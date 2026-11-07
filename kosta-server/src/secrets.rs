// kosta-server/src/secrets.rs
//! Шифрование секретов сессий «в покое».
//!
//! Ключи сессий (`auth_key` и write/mac-ключи) лежат в SQLite. Чтобы утечка
//! файла БД (бэкап, украденный диск, `SELECT *`-экспорт) не давала готовых
//! сессий, они шифруются AES-256-GCM мастер-ключом из отдельного файла.
//!
//! Формат запечатанного значения: `[MAGIC][nonce 12][ciphertext + tag 16]`.
//! Значения без MAGIC считаются легаси-записями (до шифрования) и читаются
//! как есть — при следующем сохранении сессия переедет в зашифрованный вид.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use rand::{rngs::OsRng, RngCore};

/// Версия формата зашифрованного значения.
pub const MAGIC: u8 = 0x01;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

pub struct SecretsBox {
    key: [u8; KEY_LEN],
}

impl SecretsBox {
    /// Конструктор для тестов и для случая, когда ключ уже есть в памяти.
    #[cfg(test)]
    pub fn from_key(key: [u8; KEY_LEN]) -> Self {
        Self { key }
    }

    /// Читает мастер-ключ из файла или создаёт новый (32 случайных байта).
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let bytes = std::fs::read(path)
                .with_context(|| format!("не удалось прочитать мастер-ключ {}", path.display()))?;
            let key: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                anyhow!(
                    "мастер-ключ {} должен быть ровно {} байт, а он {} байт",
                    path.display(),
                    KEY_LEN,
                    bytes.len()
                )
            })?;
            Ok(Self { key })
        } else {
            let mut key = [0u8; KEY_LEN];
            OsRng.fill_bytes(&mut key);
            std::fs::write(path, key).with_context(|| {
                format!("не удалось записать мастер-ключ {}", path.display())
            })?;
            restrict_permissions(path)?;
            Ok(Self { key })
        }
    }

    /// Шифрует секрет: возвращает `[MAGIC][nonce][ciphertext+tag]`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = kosta_crypto::gcm::encrypt_gcm(&self.key, &nonce, plaintext)
            .map_err(|e| anyhow!("ошибка шифрования секрета: {}", e))?;

        let mut sealed = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        sealed.push(MAGIC);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    /// Расшифровывает значение известной длины. Легаси-запись (без MAGIC)
    /// возвращается как есть.
    pub fn open_fixed<const N: usize>(&self, blob: &[u8]) -> Result<[u8; N]> {
        let plain = if blob.first() == Some(&MAGIC) {
            let expected = 1 + NONCE_LEN + N + TAG_LEN;
            if blob.len() != expected {
                return Err(anyhow!(
                    "неверная длина зашифрованного секрета: {} (ожидалось {})",
                    blob.len(),
                    expected
                ));
            }
            let nonce: [u8; NONCE_LEN] = blob[1..1 + NONCE_LEN]
                .try_into()
                .map_err(|_| anyhow!("битый nonce"))?;
            kosta_crypto::gcm::decrypt_gcm(&self.key, &nonce, &blob[1 + NONCE_LEN..])
                .map_err(|e| anyhow!("не удалось расшифровать секрет (другой мастер-ключ?): {}", e))?
        } else {
            // Легаси: до шифрования секрет лежал открытым текстом.
            blob.to_vec()
        };

        plain.as_slice().try_into().map_err(|_| {
            anyhow!(
                "секрет должен быть {} байт, получено {}",
                N,
                plain.len()
            )
        })
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("не удалось выставить права 0600 на {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    // На Windows ограничение прав файла требует отдельной работы с ACL;
    // ключ должен лежать вне каталога, доступного другим пользователям.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let secrets = SecretsBox::from_key([9u8; KEY_LEN]);
        let plaintext = [0xABu8; 256];
        let sealed = secrets.seal(&plaintext).unwrap();

        assert_eq!(sealed[0], MAGIC);
        assert_ne!(
            &sealed[1..],
            &plaintext[..],
            "секрет не должен читаться в открытом виде"
        );
        assert_eq!(sealed.len(), 1 + NONCE_LEN + plaintext.len() + TAG_LEN);

        let opened: [u8; 256] = secrets.open_fixed(&sealed).unwrap();
        assert_eq!(opened, plaintext);
    }

    #[test]
    fn tampered_or_foreign_secrets_are_rejected() {
        let secrets = SecretsBox::from_key([1u8; KEY_LEN]);
        let sealed = secrets.seal(&[7u8; 32]).unwrap();

        // Повреждённый шифротекст
        let mut broken = sealed.clone();
        let last = broken.len() - 1;
        broken[last] ^= 0xFF;
        assert!(secrets.open_fixed::<32>(&broken).is_err());

        // Другой мастер-ключ
        let other = SecretsBox::from_key([2u8; KEY_LEN]);
        assert!(other.open_fixed::<32>(&sealed).is_err());

        // Неверная длина
        assert!(secrets.open_fixed::<64>(&sealed).is_err());
    }

    #[test]
    fn legacy_plaintext_is_still_readable() {
        let secrets = SecretsBox::from_key([3u8; KEY_LEN]);
        let legacy = [0x5Au8; 32];
        assert_eq!(secrets.open_fixed::<32>(&legacy).unwrap(), legacy);
    }
}
