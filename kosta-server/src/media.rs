// kosta-server/src/media.rs
//! Хранилище загруженных файлов.
//!
//! Файлы лежат вне каталога статики, в `<media.dir>/<dialog_id>/<random>.<ext>`:
//! ключ хранения не содержит исходного имени, поэтому имя пользователя не
//! может выйти за пределы каталога, а расширение берётся из белого списка MIME,
//! а не из присланного имени.
//!
//! Публичный контракт для остального сервера:
//! - [`init`] — инициализация из конфига (идемпотентно);
//! - [`delete_dialog`] — best-effort очистка каталога диалога.
//!
//! Тесты работают с локальным [`MediaStorage`] во временном каталоге и не
//! трогают глобальный экземпляр.

use anyhow::{anyhow, Context, Result};
use rand::RngCore;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Максимальная длина исходного имени файла в символах (не байтах).
const MAX_FILENAME_CHARS: usize = 128;

/// Длина случайной части имени файла: 16 байт в hex.
const RANDOM_HEX_CHARS: usize = 32;

/// Лимит размера файла по умолчанию, если [`init`] ещё не вызывали.
const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Каталог аватаров внутри media-хранилища.
const AVATAR_DIR: &str = "avatars";
/// Максимальный размер картинки профиля.
const AVATAR_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// MIME-тип из белого списка и безопасное расширение на диске.
///
/// Расширение никогда не берётся из имени, присланного клиентом: иначе файл
/// `payload.html` лёг бы на диск как HTML и мог быть отдан браузеру.
pub(crate) fn extension_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "application/pdf" => Some("pdf"),
        "text/plain" => Some("txt"),
        "application/zip" => Some("zip"),
        "video/mp4" => Some("mp4"),
        // Голосовые сообщения: MediaRecorder отдаёт webm/opus в Chrome и
        // Firefox, mp4/aac в Safari, ogg/opus в некоторых сборках.
        "audio/webm" => Some("webm"),
        "audio/ogg" => Some("ogg"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "audio/wav" => Some("wav"),
        _ => None,
    }
}

/// Есть ли MIME-тип в белом списке.
pub(crate) fn is_supported_mime(mime: &str) -> bool {
    extension_for_mime(mime).is_some()
}

/// Безопасное имя файла для отдачи клиенту и хранения в БД.
///
/// Берём только basename (после `/` и `\`), выкидываем управляющие символы,
/// обрезаем пробелы и длину. Пустое имя, `.` и `..` — ошибка: такое имя
/// бессмысленно и опасно показывать/использовать в заголовках.
pub(crate) fn sanitize_filename(raw: &str) -> Result<String> {
    // `rsplit` по обоим разделителям: на Windows клиент может прислать
    // `C:\Users\me\photo.png`, на Unix — `../../etc/passwd`.
    let basename = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = basename.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(anyhow!("invalid file name"));
    }

    let truncated: String = cleaned.chars().take(MAX_FILENAME_CHARS).collect();
    Ok(truncated)
}

/// Результат сохранения файла: ключ хранения и безопасное имя для API.
#[derive(Debug, Clone)]
pub(crate) struct SavedMedia {
    /// Ключ вида `<dialog_id>/<32 hex>.<ext>`.
    pub key: String,
    /// Санитизированное исходное имя (для `media_name` и Content-Disposition).
    pub name: String,
}

/// Локальное хранилище: корневой каталог и лимит одного файла.
///
/// Отдельный экземпляр нужен тестам: глобальный [`init`] во временном каталоге
/// трогать нельзя, он общий на процесс.
#[derive(Debug)]
pub(crate) struct MediaStorage {
    dir: PathBuf,
    max_bytes: u64,
}

impl MediaStorage {
    /// Создаёт каталог (если его нет) и запоминает лимит.
    ///
    /// `max_upload_mb == 0` трактуется как 1 МиБ: нулевой лимит сделал бы
    /// загрузку невозможной и, скорее всего, означает ошибку конфигурации.
    pub(crate) fn new(dir: impl Into<PathBuf>, max_upload_mb: u64) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| {
            format!("failed to create media directory {}", dir.display())
        })?;
        Ok(Self {
            dir,
            max_bytes: max_upload_mb.max(1).saturating_mul(1024 * 1024),
        })
    }

    /// Максимальный размер одного файла в байтах.
    pub(crate) fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Корневой каталог (для тестов и диагностики).
    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Сохраняет файл и возвращает ключ хранения.
    ///
    /// Каталог диалога создаётся при необходимости; при ошибке записи
    /// недописанный файл удаляется, чтобы не копить мусор.
    pub(crate) fn save(
        &self,
        dialog_id: i64,
        bytes: &[u8],
        original_name: &str,
        mime: &str,
    ) -> Result<SavedMedia> {
        if dialog_id <= 0 {
            return Err(anyhow!("invalid dialog id"));
        }
        let extension = extension_for_mime(mime)
            .ok_or_else(|| anyhow!("unsupported media type: {}", mime))?;
        let name = sanitize_filename(original_name)?;

        if bytes.is_empty() {
            return Err(anyhow!("empty file"));
        }
        if bytes.len() as u64 > self.max_bytes {
            return Err(anyhow!(
                "file is too large: {} bytes (max {})",
                bytes.len(),
                self.max_bytes
            ));
        }

        let dialog_dir = self.dir.join(dialog_id.to_string());
        std::fs::create_dir_all(&dialog_dir).with_context(|| {
            format!("failed to create media directory {}", dialog_dir.display())
        })?;

        let file_name = format!("{}.{}", random_hex(), extension);
        let path = dialog_dir.join(&file_name);
        if let Err(e) = std::fs::write(&path, bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(anyhow::Error::new(e)
                .context(format!("failed to write media file {}", path.display())));
        }
        if let Err(e) = restrict_permissions(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }

        Ok(SavedMedia {
            key: format!("{}/{}", dialog_id, file_name),
            name,
        })
    }

    /// Читает файл по ключу хранения.
    ///
    /// Ключ проверяется до обращения к файловой системе: только
    /// `digits/32-hex.ext`, без `..`, абсолютных путей и обратных слэшей.
    pub(crate) fn read(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.storage_path(key)?;
        std::fs::read(&path)
            .map_err(|e| anyhow::Error::new(e).context(format!("failed to read media file {}", path.display())))
    }

    /// Удаляет один файл по ключу (компенсация при ошибке записи в БД).
    pub(crate) fn delete(&self, key: &str) -> Result<()> {
        let path = self.storage_path(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::Error::new(e)
                .context(format!("failed to delete media file {}", path.display()))),
        }
    }

    /// Рекурсивно удаляет каталог диалога. Отсутствие каталога — не ошибка.
    pub(crate) fn delete_dialog(&self, dialog_id: i64) -> Result<()> {
        if dialog_id <= 0 {
            return Ok(());
        }
        let dialog_dir = self.dir.join(dialog_id.to_string());
        std::fs::remove_dir_all(&dialog_dir)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .with_context(|| {
                format!("failed to delete media directory {}", dialog_dir.display())
            })
    }

    /// Сохраняет аватар пользователя и возвращает ключ хранения.
    ///
    /// Аватары живут в отдельном каталоге `avatars/` и ограничены меньшим
    /// размером, чем вложения: это картинка профиля, а не файл.
    pub(crate) fn save_avatar(&self, user_id: i64, bytes: &[u8], mime: &str) -> Result<String> {
        if user_id <= 0 {
            return Err(anyhow!("invalid user id"));
        }
        let extension = match mime {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => return Err(anyhow!("unsupported avatar type: {}", mime)),
        };
        if bytes.is_empty() {
            return Err(anyhow!("empty file"));
        }
        if bytes.len() as u64 > AVATAR_MAX_BYTES {
            return Err(anyhow!(
                "avatar is too large: {} bytes (max {})",
                bytes.len(),
                AVATAR_MAX_BYTES
            ));
        }

        let avatar_dir = self.dir.join(AVATAR_DIR);
        std::fs::create_dir_all(&avatar_dir).with_context(|| {
            format!(
                "failed to create avatar directory {}",
                avatar_dir.display()
            )
        })?;

        let file_name = format!("{}-{}.{}", user_id, random_hex(), extension);
        let path = avatar_dir.join(&file_name);
        if let Err(e) = std::fs::write(&path, bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(anyhow::Error::new(e)
                .context(format!("failed to write avatar file {}", path.display())));
        }
        if let Err(e) = restrict_permissions(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(format!("{}/{}", AVATAR_DIR, file_name))
    }

    /// Читает аватар по ключу хранения.
    pub(crate) fn read_avatar(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.avatar_path(key)?;
        std::fs::read(&path).map_err(|e| {
            anyhow::Error::new(e).context(format!("failed to read avatar {}", path.display()))
        })
    }

    /// Best-effort удаление старого аватара при замене.
    pub(crate) fn delete_avatar(&self, key: &str) {
        if let Ok(path) = self.avatar_path(key) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Проверяет ключ аватара: `avatars/<user_id>-<32 hex>.<ext>`.
    fn avatar_path(&self, key: &str) -> Result<PathBuf> {
        let (dir_part, file_part) = key
            .split_once('/')
            .ok_or_else(|| anyhow!("invalid avatar key"))?;
        if dir_part != AVATAR_DIR
            || file_part.contains("..")
            || file_part.contains('\\')
            || file_part.contains('\0')
        {
            return Err(anyhow!("invalid avatar key"));
        }
        let (stem, extension) = file_part
            .split_once('.')
            .ok_or_else(|| anyhow!("invalid avatar key"))?;
        let (user_part, hex_part) = stem
            .split_once('-')
            .ok_or_else(|| anyhow!("invalid avatar key"))?;
        if user_part.is_empty()
            || !user_part.chars().all(|c| c.is_ascii_digit())
            || user_part.parse::<i64>().map(|id| id <= 0).unwrap_or(true)
        {
            return Err(anyhow!("invalid avatar key"));
        }
        if hex_part.len() != RANDOM_HEX_CHARS || !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(anyhow!("invalid avatar key"));
        }
        if !matches!(extension, "png" | "jpg" | "webp") {
            return Err(anyhow!("invalid avatar key"));
        }
        Ok(self.dir.join(AVATAR_DIR).join(file_part))
    }

    /// Проверяет ключ и превращает его в путь внутри media-каталога.
    fn storage_path(&self, key: &str) -> Result<PathBuf> {
        if key.is_empty()
            || key.contains("..")
            || key.contains('\\')
            || key.starts_with('/')
            || key.contains('\0')
        {
            return Err(anyhow!("invalid media key"));
        }

        let mut parts = key.split('/');
        let dialog_part = parts.next().ok_or_else(|| anyhow!("invalid media key"))?;
        let file_part = parts.next().ok_or_else(|| anyhow!("invalid media key"))?;
        if parts.next().is_some() {
            return Err(anyhow!("invalid media key"));
        }

        if dialog_part.is_empty() || !dialog_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(anyhow!("invalid media key"));
        }
        let dialog_id: i64 = dialog_part
            .parse()
            .map_err(|_| anyhow!("invalid media key"))?;
        if dialog_id <= 0 {
            return Err(anyhow!("invalid media key"));
        }

        let (stem, extension) = file_part
            .split_once('.')
            .ok_or_else(|| anyhow!("invalid media key"))?;
        if stem.len() != RANDOM_HEX_CHARS || !stem.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(anyhow!("invalid media key"));
        }
        if !matches!(
            extension,
            "png" | "jpg" | "webp" | "gif" | "pdf" | "txt" | "zip" | "mp4" | "webm" | "ogg"
                | "mp3" | "m4a" | "wav"
        ) {
            return Err(anyhow!("invalid media key"));
        }

        Ok(self.dir.join(dialog_part).join(file_part))
    }
}

/// 16 случайных байт в hex: имя файла непредсказуемо и не зависит от
/// исходного имени (перебор чужих файлов по имени невозможен).
fn random_hex() -> String {
    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    raw.iter().map(|byte| format!("{:02x}", byte)).collect()
}

/// Права 0600 на файл (как у ключей): чужие пользователи ОС не прочитают.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict permissions on {}", path.display()))
}

/// На Windows аналог POSIX-прав не выставляем: каталог и так закрыт ACL профиля.
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

/// Глобальное хранилище, инициализируемое из конфига в `main`.
static STORAGE: OnceLock<MediaStorage> = OnceLock::new();

/// Инициализирует глобальное хранилище. Идемпотентно: повторный вызов
/// (например, в тестах или при перезапуске) ничего не меняет.
pub fn init(dir: &str, max_upload_mb: u64) -> Result<()> {
    if STORAGE.get().is_some() {
        return Ok(());
    }
    let storage = MediaStorage::new(dir, max_upload_mb)?;
    // Проигравший гонку просто использует уже установленный экземпляр.
    let _ = STORAGE.set(storage);
    Ok(())
}

/// Глобальный экземпляр для HTTP-хендлеров.
pub(crate) fn storage() -> Result<&'static MediaStorage> {
    STORAGE
        .get()
        .ok_or_else(|| anyhow!("media storage is not initialized"))
}

/// Лимит одного файла из глобального экземпляра.
pub fn max_bytes() -> u64 {
    STORAGE
        .get()
        .map(MediaStorage::max_bytes)
        .unwrap_or(DEFAULT_MAX_BYTES)
}

/// Best-effort удаление медиа-каталога диалога.
/// Вызывается при удалении пустого диалога: если хранилище ещё не
/// инициализировано, удалять нечего, поэтому это не ошибка.
pub fn delete_dialog(dialog_id: i64) -> Result<()> {
    match STORAGE.get() {
        Some(storage) => storage.delete_dialog(dialog_id),
        None => Ok(()),
    }
}

/// Сохраняет аватар пользователя и возвращает ключ хранения.
pub fn save_avatar(user_id: i64, bytes: &[u8], mime: &str) -> Result<String> {
    storage()?.save_avatar(user_id, bytes, mime)
}

/// Читает аватар по ключу хранения.
pub fn read_avatar(key: &str) -> Result<Vec<u8>> {
    storage()?.read_avatar(key)
}

/// Best-effort удаление старого аватара (после замены на новый).
pub fn delete_avatar(key: &str) {
    if let Some(storage) = STORAGE.get() {
        storage.delete_avatar(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Временное хранилище, не трогающее глобальный OnceLock.
    struct TempMedia {
        storage: MediaStorage,
        dir: PathBuf,
    }

    impl TempMedia {
        fn new(max_upload_mb: u64) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kosta_media_test_{}_{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            let storage = MediaStorage::new(&dir, max_upload_mb)
                .expect("не удалось создать временное media-хранилище");
            TempMedia { storage, dir }
        }
    }

    impl Drop for TempMedia {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn mime_whitelist_maps_to_safe_extensions() {
        let cases = [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/webp", "webp"),
            ("image/gif", "gif"),
            ("application/pdf", "pdf"),
            ("text/plain", "txt"),
            ("application/zip", "zip"),
            ("video/mp4", "mp4"),
        ];
        for (mime, extension) in cases {
            assert_eq!(extension_for_mime(mime), Some(extension), "MIME {mime}");
            assert!(is_supported_mime(mime));
        }

        for mime in [
            "text/html",
            "application/x-php",
            "image/svg+xml",
            "application/octet-stream",
            "",
        ] {
            assert_eq!(extension_for_mime(mime), None, "MIME {mime} не должен приниматься");
            assert!(!is_supported_mime(mime));
        }
    }

    #[test]
    fn sanitize_strips_path_separators_and_control_chars() {
        assert_eq!(sanitize_filename("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(sanitize_filename(r"..\..\secret.txt").unwrap(), "secret.txt");
        assert_eq!(sanitize_filename(r"C:\Users\me\photo.png").unwrap(), "photo.png");
        assert_eq!(sanitize_filename("  report.pdf  ").unwrap(), "report.pdf");
        assert_eq!(sanitize_filename("a\u{0}b\u{7}.txt").unwrap(), "ab.txt");
        assert_eq!(sanitize_filename("фото.png").unwrap(), "фото.png");

        let long = "a".repeat(MAX_FILENAME_CHARS + 50);
        assert_eq!(
            sanitize_filename(&long).unwrap().chars().count(),
            MAX_FILENAME_CHARS,
            "длинное имя обрезается по символам"
        );

        for bad in ["", "   ", ".", "..", "/", "///", r"..\.."] {
            assert!(
                sanitize_filename(bad).is_err(),
                "имя {bad:?} должно отвергаться"
            );
        }
    }

    #[test]
    fn save_read_delete_roundtrip() {
        let temp = TempMedia::new(25);
        let bytes = b"hello media";
        let saved = temp
            .storage
            .save(42, bytes, "photo.png", "image/png")
            .expect("save");

        let (dialog_part, file_part) = saved.key.split_once('/').expect("ключ с каталогом");
        assert_eq!(dialog_part, "42");
        let (stem, extension) = file_part.split_once('.').expect("ключ с расширением");
        assert_eq!(stem.len(), RANDOM_HEX_CHARS);
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(extension, "png", "расширение берётся из MIME, а не из имени");
        assert_eq!(saved.name, "photo.png");
        assert!(temp.storage.dir().join("42").join(file_part).is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(temp.storage.dir().join(&saved.key))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "файл должен быть доступен только владельцу");
        }

        assert_eq!(temp.storage.read(&saved.key).unwrap(), bytes);

        temp.storage.delete(&saved.key).unwrap();
        assert!(temp.storage.read(&saved.key).is_err());
        // Повторное удаление отсутствующего файла — не ошибка.
        temp.storage.delete(&saved.key).unwrap();
    }

    #[test]
    fn delete_dialog_removes_directory_and_is_idempotent() {
        let temp = TempMedia::new(25);
        temp.storage
            .save(7, b"one", "a.txt", "text/plain")
            .unwrap();
        temp.storage
            .save(7, b"two", "b.pdf", "application/pdf")
            .unwrap();
        assert!(temp.storage.dir().join("7").is_dir());

        temp.storage.delete_dialog(7).unwrap();
        assert!(!temp.storage.dir().join("7").exists());
        // Повторное удаление и несуществующий диалог — успех.
        temp.storage.delete_dialog(7).unwrap();
        temp.storage.delete_dialog(999).unwrap();
    }

    #[test]
    fn save_rejects_unknown_mime_bad_name_and_oversize() {
        let temp = TempMedia::new(1);

        assert!(
            temp.storage
                .save(1, b"x", "page.html", "text/html")
                .is_err(),
            "неизвестный MIME должен отвергаться"
        );
        assert!(
            temp.storage.save(1, b"x", "..", "text/plain").is_err(),
            "пустое/опасное имя должно отвергаться"
        );
        assert!(
            temp.storage.save(1, b"", "a.txt", "text/plain").is_err(),
            "пустой файл не сохраняется"
        );

        let too_big = vec![0u8; 1024 * 1024 + 1];
        assert!(
            temp.storage.save(1, &too_big, "big.txt", "text/plain").is_err(),
            "файл больше лимита не должен сохраняться"
        );
        assert_eq!(temp.storage.max_bytes(), 1024 * 1024);
    }

    #[test]
    fn read_rejects_keys_outside_storage() {
        let temp = TempMedia::new(25);
        let key = format!("1/{}", "a".repeat(RANDOM_HEX_CHARS));
        for bad in [
            "../etc/passwd",
            "1/../../etc/passwd",
            "/etc/passwd",
            r"1\..\x.png",
            "1/not-hex.png",
            "1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.exe",
            "x/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
            "1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png/extra",
            "0/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
            "",
        ] {
            assert!(
                temp.storage.read(bad).is_err(),
                "ключ {bad:?} должен отвергаться"
            );
        }
        // Валидный по форме, но отсутствующий файл — тоже ошибка чтения.
        assert!(temp.storage.read(&format!("{key}.png")).is_err());
    }
}
