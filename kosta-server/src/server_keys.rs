// kosta-server/src/server_keys.rs
//! Долгоживущая пара ed25519-ключей сервера.
//!
//! Приватный ключ подписывает ответы `ServerDHParamsOk`, публичный — проверяется
//! клиентом (в том числе через fingerprint в `ResPQ`). Пути настраиваются
//! переменными окружения `KOSTA_SERVER_KEY_PATH` и `KOSTA_SERVER_PUBKEY_PATH`,
//! чтобы в продакшене ключ можно было положить вне каталога запуска.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};

/// Путь к приватному ключу по умолчанию.
pub const DEFAULT_SERVER_KEY_PATH: &str = "server_key.der";
/// Путь к публичному ключу по умолчанию.
pub const DEFAULT_SERVER_PUBKEY_PATH: &str = "server_public_key.der";

/// Переменная окружения с путём к приватному ключу.
pub const SERVER_KEY_PATH_ENV: &str = "KOSTA_SERVER_KEY_PATH";
/// Переменная окружения с путём к публичному ключу.
pub const SERVER_PUBKEY_PATH_ENV: &str = "KOSTA_SERVER_PUBKEY_PATH";

/// Длина ed25519-ключа (приватного и публичного) в байтах.
const KEY_LEN: usize = 32;

/// Путь к приватному ключу сервера: env `KOSTA_SERVER_KEY_PATH` или дефолт.
pub fn server_key_path() -> PathBuf {
    std::env::var_os(SERVER_KEY_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SERVER_KEY_PATH))
}

/// Путь к публичному ключу сервера: env `KOSTA_SERVER_PUBKEY_PATH` или дефолт.
pub fn server_public_key_path() -> PathBuf {
    std::env::var_os(SERVER_PUBKEY_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SERVER_PUBKEY_PATH))
}

pub struct ServerKeypair {
    pub public: VerifyingKey,
    pub secret: SigningKey,
}

impl ServerKeypair {
    /// Загружает пару ключей из `private_path` или генерирует новую.
    ///
    /// * Если приватный ключ существует — он читается (ровно 32 байта), а
    ///   публичный файл **всегда** перезаписывается из него. Так потерянный
    ///   или устаревший `server_public_key.der` самолечится при следующем
    ///   запуске сервера.
    /// * Если приватного ключа нет — генерируется новая пара и оба файла
    ///   записываются на диск.
    ///
    /// Ошибки (нет доступа, битый или неверной длины ключ) возвращаются как
    /// `anyhow::Error`, без `unwrap`/`panic`.
    pub fn load_or_generate_at(private_path: &Path, public_path: &Path) -> Result<Self> {
        let signing_key = if private_path.exists() {
            let bytes = std::fs::read(private_path).with_context(|| {
                format!(
                    "не удалось прочитать приватный ключ {}",
                    private_path.display()
                )
            })?;
            let key_bytes: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                anyhow!(
                    "приватный ключ {} должен быть ровно {} байт, а он {} байт",
                    private_path.display(),
                    KEY_LEN,
                    bytes.len()
                )
            })?;
            // Права уже существующего ключа тоже ужесточаем: файл мог быть
            // создан старой версией сервера с дефолтными 0644.
            restrict_permissions(private_path)?;
            SigningKey::from_bytes(&key_bytes)
        } else {
            let signing_key = SigningKey::generate(&mut OsRng);
            // Создаём файл сразу с правами 0600: `fs::write` + последующий
            // chmod оставлял окно, когда приватный ключ читаем группой/всеми.
            write_private_key(private_path, signing_key.as_bytes())?;
            signing_key
        };

        let verifying_key = signing_key.verifying_key();
        // Публичный файл пишем/перезаписываем в обоих случаях: источник истины
        // всегда приватный ключ, а не то, что случайно лежит на диске.
        std::fs::write(public_path, verifying_key.as_bytes()).with_context(|| {
            format!(
                "не удалось записать публичный ключ {}",
                public_path.display()
            )
        })?;

        Ok(ServerKeypair {
            public: verifying_key,
            secret: signing_key,
        })
    }

    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let signature: Signature = self.secret.sign(data);
        signature.to_bytes().to_vec()
    }
}

/// Создаёт приватный ключ с правами 0600 сразу, без окна «сначала 0644».
#[cfg(unix)]
fn write_private_key(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("не удалось создать приватный ключ {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("не удалось записать приватный ключ {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("не удалось синхронизировать приватный ключ {}", path.display()))?;
    Ok(())
}

/// На не-unix платформах права файла задаются ACL, отдельной работы требуют.
#[cfg(not(unix))]
fn write_private_key(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)
        .with_context(|| format!("не удалось записать приватный ключ {}", path.display()))?;
    Ok(())
}

/// Ограничивает доступ к приватному ключу (0600).
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

/// Глобальная пара ключей, инициализируемая один раз при старте сервера.
static SERVER_KEYPAIR: OnceLock<ServerKeypair> = OnceLock::new();

/// Замок на время инициализации: не даёт двум потокам одновременно
/// сгенерировать/перезаписать файлы ключей (иначе приватный и публичный
/// файлы могли бы остаться от разных пар).
static INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Инициализирует глобальную пару ключей сервера.
///
/// Идемпотентно: повторный успешный вызов ничего не делает. Ошибка загрузки
/// или генерации возвращается вызывающему, а не превращается в панику
/// (как было с `lazy_static! { ... .unwrap() }`).
pub fn init_server_key() -> Result<()> {
    if SERVER_KEYPAIR.get().is_some() {
        return Ok(());
    }
    // Отрава в замке не должна превращаться в панику: просто продолжаем.
    let _guard = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if SERVER_KEYPAIR.get().is_some() {
        return Ok(());
    }
    let keypair =
        ServerKeypair::load_or_generate_at(&server_key_path(), &server_public_key_path())?;
    // Если параллельный вызов успел инициализировать глобальный ключ раньше —
    // используется он, а свой keypair просто отбрасывается.
    let _ = SERVER_KEYPAIR.set(keypair);
    Ok(())
}

/// Возвращает инициализированную пару ключей сервера.
///
/// Пока [`init_server_key`] не вызван (или завершился ошибкой), возвращает
/// `Err`: сервер не может подписывать ответы без ключа.
pub fn get_server_keypair() -> Result<&'static ServerKeypair> {
    SERVER_KEYPAIR.get().ok_or_else(|| {
        anyhow!("пара ключей сервера не инициализирована: сначала вызовите init_server_key()")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Создаёт уникальный временный каталог для теста.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kosta_server_keys_{}_{}_{}",
            tag,
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_or_generate_roundtrip() {
        let dir = temp_dir("roundtrip");
        let private = dir.join("server_key.der");
        let public = dir.join("server_public_key.der");

        let first = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        assert!(private.exists(), "приватный ключ должен быть записан");
        assert!(public.exists(), "публичный ключ должен быть записан");
        assert_eq!(
            std::fs::read(&private).unwrap(),
            first.secret.as_bytes().to_vec(),
            "в файле должен лежать именно приватный ключ"
        );
        assert_eq!(
            std::fs::read(&public).unwrap(),
            first.public.as_bytes().to_vec(),
            "в файле должен лежать именно публичный ключ"
        );

        // Повторная загрузка не генерирует новый ключ, а читает прежний.
        let second = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        assert_eq!(first.public, second.public);
        assert_eq!(first.secret.as_bytes(), second.secret.as_bytes());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_public_key_is_recreated_from_private_key() {
        let dir = temp_dir("recreate_pub");
        let private = dir.join("server_key.der");
        let public = dir.join("server_public_key.der");

        let first = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        // Имитируем потерю/удаление публичного файла.
        std::fs::remove_file(&public).unwrap();

        let second = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        assert!(public.exists(), "публичный ключ должен быть создан заново");
        assert_eq!(first.public, second.public);
        assert_eq!(
            std::fs::read(&public).unwrap(),
            second.public.as_bytes().to_vec()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_public_key_is_healed_from_private_key() {
        let dir = temp_dir("heal_pub");
        let private = dir.join("server_key.der");
        let public = dir.join("server_public_key.der");

        let first = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        // Публичный файл устарел или повреждён — он должен быть перезаписан.
        std::fs::write(&public, [0u8; KEY_LEN]).unwrap();

        let second = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        assert_eq!(first.public, second.public);
        assert_eq!(
            std::fs::read(&public).unwrap(),
            first.public.as_bytes().to_vec(),
            "публичный файл должен быть восстановлен из приватного ключа"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn private_key_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("perms");
        let private = dir.join("server_key.der");
        let public = dir.join("server_public_key.der");

        let _ = ServerKeypair::load_or_generate_at(&private, &public).unwrap();
        let mode = std::fs::metadata(&private).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "приватный ключ должен быть доступен только владельцу");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
