// kosta/src/server_keys.rs
//! Публичный ключ сервера на стороне клиента.
//!
//! Ключ читается из файла (путь: env `KOSTA_SERVER_PUBKEY_PATH`, по умолчанию
//! `server_public_key.der`). Случайная генерация здесь принципиально не
//! используется: иначе при отсутствии файла клиент проверял бы подпись
//! «своим» ключом и падал с непонятным `fingerprint mismatch` вместо
//! внятной ошибки «скопируйте ключ с сервера».

use ed25519_dalek::VerifyingKey;
use std::path::{Path, PathBuf};

use crate::error::KostaError;

/// Путь к публичному ключу сервера по умолчанию.
pub const DEFAULT_SERVER_PUBKEY_PATH: &str = "server_public_key.der";

/// Переменная окружения с путём к публичному ключу сервера.
pub const SERVER_PUBKEY_PATH_ENV: &str = "KOSTA_SERVER_PUBKEY_PATH";

/// Длина публичного ключа ed25519 в байтах.
const PUBLIC_KEY_LEN: usize = 32;

/// Читает публичный ключ сервера из указанного файла.
///
/// Файл должен содержать ровно 32 байта (raw ed25519 public key). Отдельная
/// функция нужна, чтобы тесты не трогали глобальные env-переменные.
pub fn load_public_key_from(path: &Path) -> Result<VerifyingKey, KostaError> {
    if !path.exists() {
        return Err(KostaError::ConnectionFailed(format!(
            "не найден публичный ключ сервера {}: скопируйте server_public_key.der с сервера",
            path.display()
        )));
    }

    let bytes = std::fs::read(path).map_err(|e| {
        KostaError::ConnectionFailed(format!(
            "не удалось прочитать публичный ключ сервера {}: {}; скопируйте server_public_key.der с сервера",
            path.display(),
            e
        ))
    })?;

    if bytes.len() != PUBLIC_KEY_LEN {
        return Err(KostaError::Protocol(format!(
            "публичный ключ сервера {} должен быть ровно {} байт, а он {} байт; скопируйте server_public_key.der с сервера",
            path.display(),
            PUBLIC_KEY_LEN,
            bytes.len()
        )));
    }

    let key_bytes: [u8; PUBLIC_KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
        KostaError::Protocol(format!(
            "публичный ключ сервера {} имеет неверную длину",
            path.display()
        ))
    })?;

    VerifyingKey::from_bytes(&key_bytes).map_err(|e| {
        KostaError::Protocol(format!(
            "некорректный публичный ключ сервера {}: {}; скопируйте server_public_key.der с сервера",
            path.display(),
            e
        ))
    })
}

/// Загружает публичный ключ сервера из файла `server_public_key.der`
/// (или из пути в env `KOSTA_SERVER_PUBKEY_PATH`).
///
/// Ошибка возвращается, а не превращается в панику: без настоящего ключа
/// сервера проверка подписи всё равно невозможна.
pub fn get_server_public_key() -> Result<VerifyingKey, KostaError> {
    let path = std::env::var_os(SERVER_PUBKEY_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SERVER_PUBKEY_PATH));
    load_public_key_from(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    /// Создаёт уникальный временный каталог для теста.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kosta_client_keys_{}_{}_{}",
            tag,
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = temp_dir("missing");
        let path = dir.join("server_public_key.der");
        let err = load_public_key_from(&path).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("не найден публичный ключ сервера"),
            "ожидалось понятное сообщение, получено: {}",
            msg
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_length_is_an_error() {
        let dir = temp_dir("wrong_len");
        let path = dir.join("server_public_key.der");
        std::fs::write(&path, [7u8; PUBLIC_KEY_LEN - 1]).unwrap();

        let err = load_public_key_from(&path).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("должен быть ровно 32 байт"),
            "ожидалось сообщение о длине, получено: {}",
            msg
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_public_key_is_loaded() {
        let dir = temp_dir("valid");
        let path = dir.join("server_public_key.der");

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key();
        std::fs::write(&path, public_key.as_bytes()).unwrap();

        let loaded = load_public_key_from(&path).unwrap();
        assert_eq!(loaded, public_key);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
