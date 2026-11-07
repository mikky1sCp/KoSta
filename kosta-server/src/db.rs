// kosta-server/src/db.rs
use anyhow::{anyhow, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;
use std::sync::Arc;

use crate::secrets::SecretsBox;

// структура для полной сессии (дополнена send_seq_no и msg_id_counter)
#[derive(Debug, Clone)]
pub struct FullSession {
    pub auth_key_id: i64,
    pub user_id: i64,
    pub server_salt: i64,
    pub nonce: [u8; 16],
    pub server_nonce: [u8; 16],
    pub new_nonce: [u8; 32],
    pub recv_seq_no: i32,
    pub auth_key: [u8; 256],
    pub client_write_key: [u8; 32],
    pub client_mac_key: [u8; 32],
    pub server_write_key: [u8; 32],
    pub server_mac_key: [u8; 32],
    pub last_msg_id: i64,
    pub send_counter: u32,
    pub recv_counter: u32,
    pub send_seq_no: i32,
    pub msg_id_counter: u32,
}

pub struct Database {
    pool: Pool<SqliteConnectionManager>,
    /// Мастер-ключ для шифрования секретов сессий в БД.
    secrets: Arc<SecretsBox>,
}

/// Строка истории сообщений.
pub type HistoryRow = (
    i64,
    i64,
    String,
    i64,
    bool,
    bool,
    bool,
    Option<String>,
    Option<String>,
    Option<i64>,
    bool,
);

/// Строка списка чатов для web-API: заголовок уже разрешён
/// (телефон собеседника для личного диалога, название для группы).
#[derive(Debug, Clone)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub is_group: bool,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
    /// Непрочитанные входящие сообщения именно этого зрителя.
    pub unread_count: i64,
    /// Собеседник личного диалога; у группы всегда `None`.
    pub other_user_id: Option<i64>,
    /// Собеседник личного диалога сейчас онлайн (`user_status.status == 0`).
    /// Отсутствие строки в `user_status` означает offline.
    pub is_online: bool,
}

/// Участник диалога с телефоном и статусом — для `GET /api/chats/{id}`.
#[derive(Debug, Clone)]
pub struct ParticipantRow {
    pub user_id: i64,
    pub phone: String,
    pub is_admin: bool,
    pub is_online: bool,
}

/// Общий SELECT списка чатов. Плейсхолдер `?1` — зритель: все агрегаты
/// (заголовок, последнее сообщение, непрочитанные, статус) считаются одним
/// запросом, чтобы список не уходил в N+1 по истории каждого диалога.
const CHAT_ROWS_SQL: &str = "
    SELECT d.id, d.type, d.title,
           (SELECT u.phone FROM dialog_participants dp2
            JOIN users u ON u.id = dp2.user_id
            WHERE dp2.dialog_id = d.id AND dp2.user_id != ?1
            ORDER BY u.id LIMIT 1) AS other_phone,
           (SELECT dp2.user_id FROM dialog_participants dp2
            WHERE dp2.dialog_id = d.id AND dp2.user_id != ?1
            ORDER BY dp2.user_id LIMIT 1) AS other_user_id,
           (SELECT m.text FROM messages m
            WHERE m.dialog_id = d.id AND m.deleted = 0
            ORDER BY m.timestamp DESC, m.id DESC LIMIT 1) AS last_message,
           (SELECT m.timestamp FROM messages m
            WHERE m.dialog_id = d.id AND m.deleted = 0
            ORDER BY m.timestamp DESC, m.id DESC LIMIT 1) AS last_ts,
           (SELECT COUNT(*) FROM messages m
            WHERE m.dialog_id = d.id AND m.read_status = 0
              AND m.sender_id != ?1 AND m.deleted = 0) AS unread_count,
           COALESCE((SELECT 1 FROM user_status us
                     WHERE us.user_id = (SELECT dp2.user_id FROM dialog_participants dp2
                                         WHERE dp2.dialog_id = d.id AND dp2.user_id != ?1
                                         ORDER BY dp2.user_id LIMIT 1)
                       AND us.status = 0), 0) AS other_online
    FROM dialogs d
    JOIN dialog_participants dp ON dp.dialog_id = d.id
    WHERE dp.user_id = ?1";

/// Метаданные диалога, нужные обработчикам TL и web-API.
#[derive(Debug, Clone)]
pub struct DialogMeta {
    pub id: i64,
    pub is_group: bool,
    pub title: String,
    pub participants: Vec<i64>,
}

/// Добавляет колонку, если её ещё нет (миграция старых баз).
fn ensure_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let mut rows = stmt.query([])?;
    let mut exists = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            exists = true;
            break;
        }
    }
    drop(rows);
    drop(stmt);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, decl),
            [],
        )?;
    }
    Ok(())
}

/// Конвертирует BLOB из БД в массив фиксированной длины, возвращая ошибку
/// вместо паники, если файл БД повреждён или подменён.
fn blob_fixed<const N: usize>(blob: &[u8], field: &str) -> Result<[u8; N]> {
    blob.try_into().map_err(|_| {
        anyhow!(
            "{}: ожидалось {} байт, а в базе {}",
            field,
            N,
            blob.len()
        )
    })
}

/// Данные сессии для записи в таблицу `sessions`.
/// Отдельная структура вместо 18 позиционных аргументов.
pub struct SessionRow<'a> {
    pub user_id: i64,
    pub server_salt: i64,
    pub nonce: &'a [u8; 16],
    pub server_nonce: &'a [u8; 16],
    pub new_nonce: &'a [u8; 32],
    pub recv_seq_no: i32,
    pub auth_key: &'a [u8; 256],
    pub client_write_key: &'a [u8; 32],
    pub client_mac_key: &'a [u8; 32],
    pub server_write_key: &'a [u8; 32],
    pub server_mac_key: &'a [u8; 32],
    pub last_msg_id: i64,
    pub send_counter: u32,
    pub recv_counter: u32,
    pub send_seq_no: i32,
    pub msg_id_counter: u32,
}

impl Database {
    pub fn new(path: &str, secrets: Arc<SecretsBox>) -> Result<Self> {
        // Внешние ключи в SQLite по умолчанию ВЫКЛЮЧЕНЫ — без этого PRAGMA
        // объявленные FOREIGN KEY не проверяются, и в БД появляются
        // диалоги/участники/сообщения со ссылками на несуществующих пользователей.
        // WAL и busy_timeout нужны, чтобы пул соединений не ловил SQLITE_BUSY.
        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA journal_mode = WAL;",
            )
        });
        let pool = Pool::new(manager)?;
        let db = Database { pool, secrets };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY,
                phone TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                status INTEGER NOT NULL DEFAULT 0,
                last_seen INTEGER
            );
            CREATE TABLE IF NOT EXISTS sessions (
                auth_key_id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                server_salt INTEGER NOT NULL,
                nonce BLOB NOT NULL,
                server_nonce BLOB NOT NULL,
                new_nonce BLOB NOT NULL,
                recv_seq_no INTEGER NOT NULL DEFAULT -1,
                auth_key BLOB NOT NULL,
                client_write_key BLOB NOT NULL,
                client_mac_key BLOB NOT NULL,
                server_write_key BLOB NOT NULL,
                server_mac_key BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                last_msg_id INTEGER NOT NULL DEFAULT 0,
                send_counter INTEGER NOT NULL DEFAULT 0,
                recv_counter INTEGER NOT NULL DEFAULT 0,
                send_seq_no INTEGER NOT NULL DEFAULT -1,
                msg_id_counter INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS dialogs (
                id INTEGER PRIMARY KEY,
                type INTEGER NOT NULL, -- 0 = private, 1 = group
                title TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dialog_participants (
                dialog_id INTEGER,
                user_id INTEGER,
                joined_at INTEGER NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (dialog_id, user_id),
                FOREIGN KEY(dialog_id) REFERENCES dialogs(id),
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dialog_id INTEGER NOT NULL,
                sender_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                is_outgoing INTEGER NOT NULL,
                read_status INTEGER NOT NULL DEFAULT 0,
                delivered_status INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(dialog_id) REFERENCES dialogs(id),
                FOREIGN KEY(sender_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS user_status (
                user_id INTEGER PRIMARY KEY,
                status INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS auth_tokens (
                token_hash TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            ",
        )?;

        // Индексы
        conn.execute_batch(
            r"
            CREATE INDEX IF NOT EXISTS idx_messages_dialog_id ON messages(dialog_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_dialog_participants_user_id ON dialog_participants(user_id);
            CREATE INDEX IF NOT EXISTS idx_auth_tokens_user_id ON auth_tokens(user_id);
            CREATE INDEX IF NOT EXISTS idx_auth_tokens_expires_at ON auth_tokens(expires_at);
            "
        )?;

        // Добавляем системного пользователя
        conn.execute(
            "INSERT OR IGNORE INTO users (id, phone, password_hash) VALUES (0, 'system', '')",
            [],
        )?;

        // ---- МИГРАЦИИ: новые колонки в существующих базах ----
        // messages: медиа и редактирование
        ensure_column(&conn, "messages", "media_path", "TEXT")?;
        ensure_column(&conn, "messages", "media_type", "TEXT")?;
        ensure_column(&conn, "messages", "edited_at", "INTEGER")?;
        ensure_column(&conn, "messages", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
        // dialog_participants: роли (создатель группы — администратор)
        ensure_column(&conn, "dialog_participants", "is_admin", "INTEGER NOT NULL DEFAULT 0")?;
        // sessions: время последнего использования (для очистки брошенных сессий)
        ensure_column(&conn, "sessions", "last_used_at", "INTEGER NOT NULL DEFAULT 0")?;

        // Для старых групп админа нет: назначаем администратором первого
        // вступившего (создателя), иначе группу нельзя будет модерировать.
        conn.execute(
            "UPDATE dialog_participants SET is_admin = 1
             WHERE is_admin = 0
               AND dialog_id IN (SELECT id FROM dialogs WHERE type = 1)
               AND rowid IN (
                   SELECT MIN(rowid) FROM dialog_participants
                   WHERE dialog_id IN (SELECT id FROM dialogs WHERE type = 1)
                   GROUP BY dialog_id
               )",
            [],
        )?;

        // У старых сессий last_used_at = 0; приравниваем к created_at,
        // чтобы очистка не удалила их на первом же запуске.
        conn.execute(
            "UPDATE sessions SET last_used_at = created_at
             WHERE last_used_at = 0 AND created_at > 0",
            [],
        )?;

        // Индекс по last_used_at создаётся только после миграции колонки.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_last_used_at ON sessions(last_used_at)",
            [],
        )?;

        Ok(())
    }

    pub fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        Ok(self.pool.get()?)
    }

    // ----- Пользователи -----
    pub fn create_user(&self, phone: &str, password: &str) -> Result<i64> {
        let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)
            .map_err(|e| anyhow!("bcrypt hash error: {}", e))?;
        let conn = self.get_conn()?;
        // INSERT OR IGNORE вместо голого INSERT: при гонке двух параллельных
        // регистраций проигравший получает понятную ошибку, а не 500 из-за
        // UNIQUE-констрейнта SQLite.
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO users (phone, password_hash) VALUES (?1, ?2)",
            rusqlite::params![phone, hashed],
        )?;
        if inserted == 0 {
            return Err(anyhow!("phone number is already registered"));
        }
        Ok(conn.last_insert_rowid())
    }

    pub fn authenticate(&self, phone: &str, password: &str) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT id, password_hash FROM users WHERE phone = ?1")?;
        let mut rows = stmt.query(rusqlite::params![phone])?;
        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let hash: String = row.get(1)?;
            // Пустой или битый хеш (системный пользователь, повреждённая запись)
            // — это неверные учётные данные, а не 500-я ошибка сервера.
            let valid = bcrypt::verify(password, &hash).unwrap_or(false);
            if valid {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    pub fn get_user_id_by_phone(&self, phone: &str) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT id FROM users WHERE phone = ?1")?;
        let mut rows = stmt.query(rusqlite::params![phone])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Точный поиск пользователя по номеру: `(user_id, phone)`.
    ///
    /// Для `GET /api/users/lookup` и создания диалогов. Сравнение строгое
    /// (`=`), а не `LIKE`: подстрочный поиск позволял бы перечислять номера.
    pub fn find_user_by_phone(&self, phone: &str) -> Result<Option<(i64, String)>> {
        let conn = self.get_conn()?;
        let found = conn
            .query_row(
                "SELECT id, phone FROM users WHERE phone = ?1",
                rusqlite::params![phone],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(found)
    }

    /// Телефон и онлайн-статус пользователя для `GET /api/me`.
    ///
    /// Статус берётся из `user_status`, а не из реестра WebSocket-соединений:
    /// нет строки — значит offline.
    pub fn get_user_profile(&self, user_id: i64) -> Result<Option<(String, bool)>> {
        let conn = self.get_conn()?;
        let profile = conn
            .query_row(
                "SELECT u.phone, COALESCE(us.status = 0, 0)
                 FROM users u
                 LEFT JOIN user_status us ON us.user_id = u.id
                 WHERE u.id = ?1",
                rusqlite::params![user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        Ok(profile)
    }

    // ----- Токены сессий (web-API и WebSocket) -----

    /// Время жизни токена.
    pub const TOKEN_TTL_SECS: i64 = 7 * 24 * 60 * 60;

    /// Выпускает новый токен и возвращает его в открытом виде.
    /// В базе хранится только SHA-256 от токена: утечка БД не даёт готовых сессий.
    pub fn create_auth_token(&self, user_id: i64) -> Result<String> {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token: String = raw.iter().map(|b| format!("{:02x}", b)).collect();

        // Попутно чистим просроченные токены, чтобы таблица не росла бесконечно.
        let _ = self.purge_expired_tokens();

        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO auth_tokens (token_hash, user_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                hash_token(&token),
                user_id,
                now_secs(),
                now_secs() + Self::TOKEN_TTL_SECS
            ],
        )?;
        Ok(token)
    }

    /// Возвращает user_id по токену, если он существует и не истёк.
    pub fn user_id_by_token(&self, token: &str) -> Result<Option<i64>> {
        if token.is_empty() {
            return Ok(None);
        }
        let conn = self.get_conn()?;
        let mut stmt = conn
            .prepare("SELECT user_id FROM auth_tokens WHERE token_hash = ?1 AND expires_at > ?2")?;
        let mut rows = stmt.query(rusqlite::params![hash_token(token), now_secs()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Отзывает конкретный токен (logout).
    pub fn revoke_token(&self, token: &str) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "DELETE FROM auth_tokens WHERE token_hash = ?1",
            rusqlite::params![hash_token(token)],
        )?;
        Ok(())
    }

    /// Удаляет просроченные токены, возвращает сколько удалено.
    pub fn purge_expired_tokens(&self) -> Result<usize> {
        let conn = self.get_conn()?;
        let removed = conn.execute(
            "DELETE FROM auth_tokens WHERE expires_at <= ?1",
            rusqlite::params![now_secs()],
        )?;
        Ok(removed)
    }

    // ----- Сессии -----
    pub fn save_session_full(&self, auth_key_id: i64, row: &SessionRow<'_>) -> Result<()> {
        // Секреты сессии больше не попадают в БД открытым текстом.
        let sealed_auth_key = self.secrets.seal(row.auth_key)?;
        let sealed_cwk = self.secrets.seal(row.client_write_key)?;
        let sealed_cmk = self.secrets.seal(row.client_mac_key)?;
        let sealed_swk = self.secrets.seal(row.server_write_key)?;
        let sealed_smk = self.secrets.seal(row.server_mac_key)?;

        let conn = self.get_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO sessions
             (auth_key_id, user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
              auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key, created_at,
              last_msg_id, send_counter, recv_counter, send_seq_no, msg_id_counter, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now'),
                     ?13, ?14, ?15, ?16, ?17, strftime('%s','now'))",
            rusqlite::params![
                auth_key_id, row.user_id, row.server_salt,
                row.nonce, row.server_nonce, row.new_nonce,
                row.recv_seq_no,
                sealed_auth_key,
                sealed_cwk, sealed_cmk,
                sealed_swk, sealed_smk,
                row.last_msg_id, row.send_counter, row.recv_counter,
                row.send_seq_no, row.msg_id_counter,
            ],
        )?;
        Ok(())
    }

    /// user_id сессии (для проверки авторизации).
    pub fn get_session_user_id(&self, auth_key_id: i64) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let user_id = conn
            .query_row(
                "SELECT user_id FROM sessions WHERE auth_key_id = ?1",
                rusqlite::params![auth_key_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(user_id)
    }

    /// Удаляет брошенные сессии: незавершённые рукопожатия (user_id = 0)
    /// и сессии, не использованные дольше `idle_secs`.
    /// Возвращает число удалённых строк.
    pub fn purge_stale_sessions(&self, anonymous_idle_secs: i64, idle_secs: i64) -> Result<usize> {
        let conn = self.get_conn()?;
        let now = now_secs();
        let removed = conn.execute(
            "DELETE FROM sessions
             WHERE (user_id = 0 AND last_used_at < ?1)
                OR last_used_at < ?2",
            rusqlite::params![now - anonymous_idle_secs, now - idle_secs],
        )?;
        Ok(removed)
    }

    pub fn update_session_user_id(&self, auth_key_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE sessions SET user_id = ?1 WHERE auth_key_id = ?2",
            rusqlite::params![user_id, auth_key_id],
        )?;
        Ok(())
    }

    pub fn get_full_session(&self, auth_key_id: i64) -> Result<Option<FullSession>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT auth_key_id, user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
                    auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key,
                    last_msg_id, send_counter, recv_counter, send_seq_no, msg_id_counter
             FROM sessions WHERE auth_key_id = ?1"
        )?;
        let mut rows = stmt.query(rusqlite::params![auth_key_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(full_session_from_row(row, &self.secrets)?)),
            None => Ok(None),
        }
    }

    pub fn get_full_session_for_user(&self, user_id: i64) -> Result<Option<FullSession>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT auth_key_id, user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
                    auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key,
                    last_msg_id, send_counter, recv_counter, send_seq_no, msg_id_counter
             FROM sessions WHERE user_id = ?1 ORDER BY last_used_at DESC LIMIT 1"
        )?;
        let mut rows = stmt.query(rusqlite::params![user_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(full_session_from_row(row, &self.secrets)?)),
            None => Ok(None),
        }
    }

    // ===== ДИАЛОГИ =====

    /// Проверяет, существует ли пользователь.
    pub fn user_exists(&self, user_id: i64) -> Result<bool> {
        let conn = self.get_conn()?;
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM users WHERE id = ?1",
                rusqlite::params![user_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    /// Находит или создаёт личный диалог двух пользователей.
    ///
    /// Раньше JOIN без условия `dp1.user_id != dp2.user_id` при `user1 == user2`
    /// матчил любую строку участия и возвращал диалог с третьим человеком.
    fn find_private_dialog(conn: &rusqlite::Connection, user1: i64, user2: i64) -> Result<Option<i64>> {
        let id = conn
            .query_row(
                "SELECT d.id FROM dialogs d
                 WHERE d.type = 0
                   AND (SELECT COUNT(*) FROM dialog_participants p WHERE p.dialog_id = d.id) = 2
                   AND EXISTS (SELECT 1 FROM dialog_participants p WHERE p.dialog_id = d.id AND p.user_id = ?1)
                   AND EXISTS (SELECT 1 FROM dialog_participants p WHERE p.dialog_id = d.id AND p.user_id = ?2)
                 LIMIT 1",
                rusqlite::params![user1, user2],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn create_private_dialog(&self, user1: i64, user2: i64) -> Result<i64> {
        if user1 == user2 {
            return Err(anyhow!("нельзя создать личный диалог с самим собой"));
        }
        if !self.user_exists(user1)? || !self.user_exists(user2)? {
            return Err(anyhow!("участник диалога не существует"));
        }

        let mut conn = self.get_conn()?;
        let tx = conn.transaction()?;
        if let Some(id) = Self::find_private_dialog(&tx, user1, user2)? {
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO dialogs (type, title, created_at) VALUES (0, '', strftime('%s','now'))",
            [],
        )?;
        let dialog_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at, is_admin)
             VALUES (?1, ?2, strftime('%s','now'), 0)",
            rusqlite::params![dialog_id, user1],
        )?;
        tx.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at, is_admin)
             VALUES (?1, ?2, strftime('%s','now'), 0)",
            rusqlite::params![dialog_id, user2],
        )?;
        tx.commit()?;
        Ok(dialog_id)
    }

    /// Создаёт группу. Создатель всегда становится администратором,
    /// дубликаты участников игнорируются.
    pub fn create_group_dialog(&self, title: &str, creator: i64, participants: &[i64]) -> Result<i64> {
        if !self.user_exists(creator)? {
            return Err(anyhow!("создатель группы не существует"));
        }
        let mut conn = self.get_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO dialogs (type, title, created_at) VALUES (1, ?1, strftime('%s','now'))",
            rusqlite::params![title],
        )?;
        let dialog_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at, is_admin)
             VALUES (?1, ?2, strftime('%s','now'), 1)",
            rusqlite::params![dialog_id, creator],
        )?;
        for user_id in participants {
            if *user_id == creator {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO dialog_participants (dialog_id, user_id, joined_at, is_admin)
                 VALUES (?1, ?2, strftime('%s','now'), 0)",
                rusqlite::params![dialog_id, user_id],
            )?;
        }
        tx.commit()?;
        Ok(dialog_id)
    }

    /// Возвращает диалоги пользователя с заголовком (телефон собеседника
    /// для личного диалога, название для группы), последним сообщением,
    /// числом непрочитанных и статусом собеседника.
    /// Заменяет старый `get_user_dialogs` + N+1 запросы истории в web-слое.
    pub fn get_user_chats(&self, user_id: i64) -> Result<Vec<ChatRow>> {
        let conn = self.get_conn()?;
        let sql = format!(
            "{} ORDER BY COALESCE(last_ts, d.created_at) DESC",
            CHAT_ROWS_SQL
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![user_id], chat_row_from_row)?;
        let mut chats = Vec::new();
        for row in rows {
            chats.push(row?);
        }
        Ok(chats)
    }

    /// Одна карточка чата глазами зрителя.
    ///
    /// `None` — диалога нет **или** зритель не участник. Различать эти случаи
    /// (404 против 403) должен вызывающий слой: БД не знает, что показывать.
    pub fn get_chat_row(&self, user_id: i64, dialog_id: i64) -> Result<Option<ChatRow>> {
        let conn = self.get_conn()?;
        let sql = format!("{} AND d.id = ?2 LIMIT 1", CHAT_ROWS_SQL);
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![user_id, dialog_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(chat_row_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Метаданные диалога для TL-ответов: тип, заголовок, участники.
    pub fn get_dialog_meta(&self, dialog_id: i64, viewer_id: i64) -> Result<Option<DialogMeta>> {
        let conn = self.get_conn()?;
        let base = conn
            .query_row(
                "SELECT d.type, d.title,
                        (SELECT u.phone FROM dialog_participants dp2
                         JOIN users u ON u.id = dp2.user_id
                         WHERE dp2.dialog_id = d.id AND dp2.user_id != ?2
                         ORDER BY u.id LIMIT 1) AS other_phone
                 FROM dialogs d WHERE d.id = ?1",
                rusqlite::params![dialog_id, viewer_id],
                |row| {
                    let typ: i64 = row.get(0)?;
                    let title: String = row.get(1)?;
                    let other_phone: Option<String> = row.get(2)?;
                    Ok((typ, title, other_phone))
                },
            )
            .optional()?;
        let Some((typ, raw_title, other_phone)) = base else {
            return Ok(None);
        };

        let mut stmt = conn.prepare(
            "SELECT user_id FROM dialog_participants WHERE dialog_id = ?1 ORDER BY joined_at, user_id",
        )?;
        let participants = stmt
            .query_map(rusqlite::params![dialog_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;

        let is_group = typ == 1;
        let title = dialog_title(is_group, &raw_title, other_phone.as_deref(), dialog_id);
        Ok(Some(DialogMeta {
            id: dialog_id,
            is_group,
            title,
            participants,
        }))
    }

    pub fn get_dialog_participants(&self, dialog_id: i64) -> Result<Vec<i64>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT user_id FROM dialog_participants WHERE dialog_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![dialog_id], |row| row.get(0))?;
        let mut users = Vec::new();
        for user in rows {
            users.push(user?);
        }
        Ok(users)
    }

    /// Участники диалога с телефоном, ролью и онлайн-статусом.
    ///
    /// `LEFT JOIN user_status`: пользователь без строки статуса считается
    /// offline, но не должен выпадать из списка участников (как было бы
    /// с INNER JOIN).
    pub fn get_dialog_participants_detailed(&self, dialog_id: i64) -> Result<Vec<ParticipantRow>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT dp.user_id, u.phone, dp.is_admin,
                    CASE WHEN us.status = 0 THEN 1 ELSE 0 END
             FROM dialog_participants dp
             JOIN users u ON u.id = dp.user_id
             LEFT JOIN user_status us ON us.user_id = dp.user_id
             WHERE dp.dialog_id = ?1
             ORDER BY dp.joined_at, dp.user_id",
        )?;
        let rows = stmt.query_map(rusqlite::params![dialog_id], |row| {
            Ok(ParticipantRow {
                user_id: row.get(0)?,
                phone: row.get(1)?,
                is_admin: row.get::<_, i64>(2)? != 0,
                is_online: row.get::<_, i64>(3)? != 0,
            })
        })?;
        let mut participants = Vec::new();
        for row in rows {
            participants.push(row?);
        }
        Ok(participants)
    }

    pub fn add_participant(&self, dialog_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO dialog_participants (dialog_id, user_id, joined_at, is_admin)
             VALUES (?1, ?2, strftime('%s','now'), 0)",
            rusqlite::params![dialog_id, user_id],
        )?;
        Ok(())
    }

    pub fn remove_participant(&self, dialog_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "DELETE FROM dialog_participants WHERE dialog_id = ?1 AND user_id = ?2",
            rusqlite::params![dialog_id, user_id],
        )?;
        Ok(())
    }

    /// Если в группе не осталось администраторов (например, админ вышел),
    /// назначает администратором первого участника, чтобы группа не осталась
    /// без управления.
    pub fn ensure_group_has_admin(&self, dialog_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        let admins: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dialog_participants WHERE dialog_id = ?1 AND is_admin = 1",
            rusqlite::params![dialog_id],
            |row| row.get(0),
        )?;
        if admins == 0 {
            conn.execute(
                "UPDATE dialog_participants SET is_admin = 1
                 WHERE dialog_id = ?1
                   AND rowid = (SELECT MIN(rowid) FROM dialog_participants WHERE dialog_id = ?1)",
                rusqlite::params![dialog_id],
            )?;
        }
        Ok(())
    }

    /// Удаляет диалог, если в нём не осталось участников (вместе с сообщениями).
    /// Возвращает true, если диалог был удалён.
    pub fn delete_empty_dialog(&self, dialog_id: i64) -> Result<bool> {
        let mut conn = self.get_conn()?;
        let tx = conn.transaction()?;
        let participants: i64 = tx.query_row(
            "SELECT COUNT(*) FROM dialog_participants WHERE dialog_id = ?1",
            rusqlite::params![dialog_id],
            |row| row.get(0),
        )?;
        if participants != 0 {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM messages WHERE dialog_id = ?1",
            rusqlite::params![dialog_id],
        )?;
        tx.execute("DELETE FROM dialogs WHERE id = ?1", rusqlite::params![dialog_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Id диалогов пользователя (для адресной рассылки статусов в WebSocket).
    pub fn get_user_dialog_ids(&self, user_id: i64) -> Result<Vec<i64>> {
        let conn = self.get_conn()?;
        let mut stmt =
            conn.prepare("SELECT dialog_id FROM dialog_participants WHERE user_id = ?1")?;
        let ids = stmt
            .query_map(rusqlite::params![user_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(ids)
    }

    /// Тип диалога: Some(0) — личный, Some(1) — группа, None — не существует.
    pub fn get_dialog_type(&self, dialog_id: i64) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let typ = conn
            .query_row(
                "SELECT type FROM dialogs WHERE id = ?1",
                rusqlite::params![dialog_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(typ)
    }

    /// Переименование группы. Права проверяет вызывающий слой: БД не знает,
    /// кто именно делает запрос и является ли он администратором.
    pub fn rename_dialog(&self, dialog_id: i64, title: &str) -> Result<()> {
        let conn = self.get_conn()?;
        let updated = conn.execute(
            "UPDATE dialogs SET title = ?1 WHERE id = ?2",
            rusqlite::params![title, dialog_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("диалог {} не найден", dialog_id));
        }
        Ok(())
    }

    /// Является ли пользователь администратором диалога.
    pub fn is_dialog_admin(&self, dialog_id: i64, user_id: i64) -> Result<bool> {
        let conn = self.get_conn()?;
        let is_admin: Option<i64> = conn
            .query_row(
                "SELECT is_admin FROM dialog_participants WHERE dialog_id = ?1 AND user_id = ?2",
                rusqlite::params![dialog_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(is_admin.unwrap_or(0) != 0)
    }

    /// Диалог, в котором лежит сообщение (для проверки прав на ack/редактирование).
    pub fn get_message_dialog_id(&self, message_id: i64) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let dialog_id = conn
            .query_row(
                "SELECT dialog_id FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(dialog_id)
    }

    // ===== СООБЩЕНИЯ (ОБНОВЛЁННЫЕ) =====

    /// 7 аргументов — это плоский набор полей одной строки; структура
    /// ради двух вызовов была бы шумнее.
    #[allow(clippy::too_many_arguments)]
    pub fn save_message(
        &self,
        dialog_id: i64,
        sender_id: i64,
        text: &str,
        timestamp: i64,
        is_outgoing: bool,
        media_path: Option<&str>,
        media_type: Option<&str>,
    ) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO messages (dialog_id, sender_id, text, timestamp, is_outgoing, media_path, media_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                dialog_id,
                sender_id,
                text,
                timestamp,
                if is_outgoing { 1 } else { 0 },
                media_path,
                media_type,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_history(
        &self,
        dialog_id: i64,
        offset: i32,
        limit: i32,
    ) -> Result<Vec<HistoryRow>> {
        // Защита в глубину: SQLite трактует отрицательный LIMIT как «без лимита»,
        // поэтому ограничение живёт ещё и здесь, а не только в хендлерах.
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, sender_id, text, timestamp, is_outgoing, read_status, delivered_status,
                    media_path, media_type, edited_at, deleted
             FROM messages
             WHERE dialog_id = ?1 AND deleted = 0
             ORDER BY timestamp DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![dialog_id, limit, offset],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get::<_, i32>(4)? != 0,
                    row.get::<_, i32>(5)? != 0,
                    row.get::<_, i32>(6)? != 0,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get::<_, i32>(10)? != 0,
                ))
            },
        )?;
        let mut msgs = Vec::new();
        for msg in rows {
            msgs.push(msg?);
        }
        Ok(msgs)
    }

    #[allow(dead_code)]
    pub fn mark_message_read(&self, message_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE messages SET read_status = 1 WHERE id = ?1",
            rusqlite::params![message_id],
        )?;
        Ok(())
    }

    /// Помечает прочитанными все входящие сообщения диалога для `viewer_id`
    /// и возвращает число изменённых строк.
    ///
    /// Исходящие (`sender_id == viewer_id`) и удалённые не трогаем: иначе
    /// счётчик непрочитанных у собеседника поехал бы от чужого действия.
    pub fn mark_dialog_read(&self, dialog_id: i64, viewer_id: i64) -> Result<usize> {
        let conn = self.get_conn()?;
        let marked = conn.execute(
            "UPDATE messages SET read_status = 1
             WHERE dialog_id = ?1 AND sender_id != ?2
               AND read_status = 0 AND deleted = 0",
            rusqlite::params![dialog_id, viewer_id],
        )?;
        Ok(marked)
    }

    pub fn mark_message_delivered(&self, message_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE messages SET delivered_status = 1 WHERE id = ?1",
            rusqlite::params![message_id],
        )?;
        Ok(())
    }

    // === Редактирование и удаление ===
    // Пока не вызываются: в TL-протоколе нет конструкторов EditMessage/DeleteMessage.
    // Оставлены как задел, схема (edited_at, deleted) их уже поддерживает.

    #[allow(dead_code)]
    pub fn edit_message(&self, message_id: i64, user_id: i64, new_text: &str) -> Result<()> {
        let conn = self.get_conn()?;
        // Проверяем, что пользователь является отправителем и сообщение не удалено
        let mut stmt = conn.prepare("SELECT sender_id FROM messages WHERE id = ?1 AND deleted = 0")?;
        let sender_id: i64 = stmt.query_row([message_id], |row| row.get(0))?;
        if sender_id != user_id {
            return Err(anyhow!("You are not the sender of this message"));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE messages SET text = ?1, edited_at = ?2 WHERE id = ?3",
            rusqlite::params![new_text, now, message_id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn delete_message(&self, message_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        // Проверяем, что пользователь является отправителем и сообщение не удалено
        let mut stmt = conn.prepare("SELECT sender_id FROM messages WHERE id = ?1 AND deleted = 0")?;
        let sender_id: i64 = stmt.query_row([message_id], |row| row.get(0))?;
        if sender_id != user_id {
            return Err(anyhow!("You are not the sender of this message"));
        }
        conn.execute(
            "UPDATE messages SET deleted = 1 WHERE id = ?1",
            rusqlite::params![message_id],
        )?;
        Ok(())
    }

    // ===== СТАТУСЫ ПОЛЬЗОВАТЕЛЕЙ =====

    pub fn set_user_status(&self, user_id: i64, status: i32) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO user_status (user_id, status, last_seen, updated_at)
             VALUES (?1, ?2, strftime('%s','now'), strftime('%s','now'))",
            rusqlite::params![user_id, status],
        )?;
        Ok(())
    }
}

/// Человекочитаемый заголовок диалога.
///
/// Для личного диалога — телефон собеседника, для группы — сохранённое
/// название, а если его нет (старые записи) — "Группа {id}".
fn dialog_title(is_group: bool, raw_title: &str, other_phone: Option<&str>, dialog_id: i64) -> String {
    if is_group {
        let title = raw_title.trim();
        if title.is_empty() {
            format!("Группа {}", dialog_id)
        } else {
            title.to_string()
        }
    } else {
        other_phone
            .map(|p| p.to_string())
            .unwrap_or_else(|| format!("Диалог {}", dialog_id))
    }
}

/// Собирает `ChatRow` из строки `CHAT_ROWS_SQL`.
///
/// У группы «собеседника» нет, даже если подзапрос вернул случайного
/// участника: иначе фронтенд показал бы статус произвольного человека.
fn chat_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    let id: i64 = row.get(0)?;
    let typ: i64 = row.get(1)?;
    let raw_title: String = row.get(2)?;
    let other_phone: Option<String> = row.get(3)?;
    let other_user_id: Option<i64> = row.get(4)?;
    let last_message: Option<String> = row.get(5)?;
    let last_timestamp: Option<i64> = row.get(6)?;
    let unread_count: i64 = row.get(7)?;
    let other_online: i64 = row.get(8)?;

    let is_group = typ == 1;
    let (other_user_id, is_online) = if is_group {
        (None, false)
    } else {
        (other_user_id, other_online != 0)
    };

    Ok(ChatRow {
        id,
        title: dialog_title(is_group, &raw_title, other_phone.as_deref(), id),
        is_group,
        last_message,
        last_timestamp,
        unread_count,
        other_user_id,
        is_online,
    })
}

/// Собирает `FullSession` из строки `sessions` с понятными ошибками
/// вместо `copy_from_slice`-паник на повреждённой БД.
fn full_session_from_row(row: &rusqlite::Row<'_>, secrets: &SecretsBox) -> Result<FullSession> {
    let auth_key_id: i64 = row.get(0)?;
    let user_id: i64 = row.get(1)?;
    let server_salt: i64 = row.get(2)?;
    let nonce: Vec<u8> = row.get(3)?;
    let server_nonce: Vec<u8> = row.get(4)?;
    let new_nonce: Vec<u8> = row.get(5)?;
    let recv_seq_no: i32 = row.get(6)?;
    let auth_key: Vec<u8> = row.get(7)?;
    let client_write_key: Vec<u8> = row.get(8)?;
    let client_mac_key: Vec<u8> = row.get(9)?;
    let server_write_key: Vec<u8> = row.get(10)?;
    let server_mac_key: Vec<u8> = row.get(11)?;
    let last_msg_id: i64 = row.get(12)?;
    let send_counter: i64 = row.get(13)?;
    let recv_counter: i64 = row.get(14)?;
    let send_seq_no: i32 = row.get(15)?;
    let msg_id_counter: i64 = row.get(16)?;

    Ok(FullSession {
        auth_key_id,
        user_id,
        server_salt,
        nonce: blob_fixed(&nonce, "sessions.nonce")?,
        server_nonce: blob_fixed(&server_nonce, "sessions.server_nonce")?,
        new_nonce: blob_fixed(&new_nonce, "sessions.new_nonce")?,
        recv_seq_no,
        // Секреты расшифровываются мастер-ключом; легаси-записи читаются как есть.
        auth_key: secrets.open_fixed(&auth_key)?,
        client_write_key: secrets.open_fixed(&client_write_key)?,
        client_mac_key: secrets.open_fixed(&client_mac_key)?,
        server_write_key: secrets.open_fixed(&server_write_key)?,
        server_mac_key: secrets.open_fixed(&server_mac_key)?,
        last_msg_id,
        send_counter: send_counter as u32,
        recv_counter: recv_counter as u32,
        send_seq_no,
        msg_id_counter: msg_id_counter as u32,
    })
}

/// Текущее время в unix-секундах.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// SHA-256 в hex — то, что реально лежит в таблице auth_tokens.
fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Временная БД в файле: пул соединений с `:memory:` дал бы каждому
    /// соединению свою пустую базу, поэтому используем файл во временном каталоге.
    struct TempDb {
        db: Database,
        path: std::path::PathBuf,
    }

    impl TempDb {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "kosta_test_{}.db",
                rand::random::<u64>()
            ));
            let secrets = Arc::new(SecretsBox::from_key([0x42u8; 32]));
            let db = Database::new(path.to_str().expect("temp path is utf-8"), secrets)
                .expect("не удалось открыть тестовую БД");
            TempDb { db, path }
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{}", self.path.display(), suffix));
            }
        }
    }

    #[test]
    fn token_roundtrip_and_revoke() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+1000", "pass").unwrap();

        let token = t.db.create_auth_token(user_id).unwrap();
        assert_eq!(token.len(), 64, "токен — 32 случайных байта в hex");
        assert_eq!(t.db.user_id_by_token(&token).unwrap(), Some(user_id));

        // Отозванный токен больше не принимается
        t.db.revoke_token(&token).unwrap();
        assert_eq!(t.db.user_id_by_token(&token).unwrap(), None);

        // Чужой/мусорный токен — тоже нет
        assert_eq!(t.db.user_id_by_token("deadbeef").unwrap(), None);
        assert_eq!(t.db.user_id_by_token("").unwrap(), None);
    }

    #[test]
    fn expired_token_is_rejected_and_purged() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+1001", "pass").unwrap();
        let token = t.db.create_auth_token(user_id).unwrap();

        // Просрочиваем токен вручную
        t.db.get_conn()
            .unwrap()
            .execute(
                "UPDATE auth_tokens SET expires_at = ?1",
                rusqlite::params![now_secs() - 1],
            )
            .unwrap();

        assert_eq!(t.db.user_id_by_token(&token).unwrap(), None);
        assert_eq!(t.db.purge_expired_tokens().unwrap(), 1);
    }

    #[test]
    fn wrong_password_does_not_create_new_user() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+1002", "correct").unwrap();

        // Пароль неверный: authenticate возвращает None, но пользователь существует,
        // значит логин-хендлер обязан ответить 401, а не регистрировать нового.
        assert_eq!(t.db.authenticate("+1002", "wrong").unwrap(), None);
        assert_eq!(t.db.get_user_id_by_phone("+1002").unwrap(), Some(user_id));
        assert_eq!(t.db.authenticate("+1002", "correct").unwrap(), Some(user_id));
    }

    #[test]
    fn session_secrets_are_encrypted_at_rest() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+1003", "pass").unwrap();

        let auth_key = [0xABu8; 256];
        let client_write_key = [0xCDu8; 32];
        t.db.save_session_full(
            12345,
            &SessionRow {
                user_id,
                server_salt: 0,
                nonce: &[1u8; 16],
                server_nonce: &[2u8; 16],
                new_nonce: &[3u8; 32],
                recv_seq_no: -1,
                auth_key: &auth_key,
                client_write_key: &client_write_key,
                client_mac_key: &[5u8; 32],
                server_write_key: &[6u8; 32],
                server_mac_key: &[7u8; 32],
                last_msg_id: 0,
                send_counter: 0,
                recv_counter: 0,
                send_seq_no: -1,
                msg_id_counter: 0,
            },
        )
        .unwrap();

        // В файле БД не должно быть ключей открытым текстом.
        // WAL держит свежие страницы в отдельном файле — сначала checkpoint.
        t.db.get_conn()
            .unwrap()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let raw = std::fs::read(&t.path).unwrap();
        let has_long_run = |byte: u8| raw.windows(32).any(|w| w.iter().all(|&b| b == byte));
        assert!(!has_long_run(0xAB), "auth_key лежит в БД открытым текстом");
        assert!(
            !has_long_run(0xCD),
            "write-ключ лежит в БД открытым текстом"
        );

        // При этом сессия читается обратно без потерь
        let session = t.db.get_full_session(12345).unwrap().unwrap();
        assert_eq!(session.auth_key, auth_key);
        assert_eq!(session.client_write_key, client_write_key);
        assert_eq!(session.user_id, user_id);
        assert_eq!(t.db.get_session_user_id(12345).unwrap(), Some(user_id));
    }

    /// Сохранение тестовой сессии с минимальной обвязкой.
    fn save_test_session(db: &Database, auth_key_id: i64, user_id: i64) {
        db.save_session_full(
            auth_key_id,
            &SessionRow {
                user_id,
                server_salt: 0,
                nonce: &[1u8; 16],
                server_nonce: &[2u8; 16],
                new_nonce: &[3u8; 32],
                recv_seq_no: -1,
                auth_key: &[0xAAu8; 256],
                client_write_key: &[0xBBu8; 32],
                client_mac_key: &[0xCCu8; 32],
                server_write_key: &[0xDDu8; 32],
                server_mac_key: &[0xEEu8; 32],
                last_msg_id: 0,
                send_counter: 0,
                recv_counter: 0,
                send_seq_no: -1,
                msg_id_counter: 0,
            },
        )
        .unwrap();
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+2000", "pass").unwrap();

        // Участник с несуществующим user_id должен отвергаться БД, а не создавать мусор.
        let conn = t.db.get_conn().unwrap();
        let orphan_participant = conn.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (1, 999999, 0)",
            [],
        );
        assert!(
            orphan_participant.is_err(),
            "PRAGMA foreign_keys=ON должен запрещать участников-призраков"
        );

        // Сообщение в несуществующий диалог тоже не должно сохраняться.
        assert!(t.db.save_message(999999, user_id, "text", 0, true, None, None).is_err());
    }

    #[test]
    fn private_dialog_with_self_is_rejected() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+2001", "pass").unwrap();
        assert!(t.db.create_private_dialog(user_id, user_id).is_err());
    }

    #[test]
    fn private_dialog_is_reused_and_has_exactly_two_participants() {
        let t = TempDb::new();
        let u1 = t.db.create_user("+2002", "pass").unwrap();
        let u2 = t.db.create_user("+2003", "pass").unwrap();

        let first = t.db.create_private_dialog(u1, u2).unwrap();
        let second = t.db.create_private_dialog(u1, u2).unwrap();
        assert_eq!(first, second, "повторный вызов должен вернуть тот же диалог");

        let mut participants = t.db.get_dialog_participants(first).unwrap();
        participants.sort();
        assert_eq!(participants, vec![u1, u2]);
        assert_eq!(t.db.get_dialog_type(first).unwrap(), Some(0));
    }

    #[test]
    fn group_type_title_and_admin_roles() {
        let t = TempDb::new();
        let creator = t.db.create_user("+2004", "pass").unwrap();
        let member = t.db.create_user("+2005", "pass").unwrap();

        let dialog_id = t
            .db
            .create_group_dialog("  Тестовая группа  ", creator, &[member])
            .unwrap();

        let meta = t.db.get_dialog_meta(dialog_id, creator).unwrap().unwrap();
        assert!(meta.is_group, "тип берётся из dialogs.type, а не из числа участников");
        assert_eq!(meta.title, "Тестовая группа", "название группы должно сохраняться");
        assert_eq!(meta.participants.len(), 2);
        assert!(t.db.is_dialog_admin(dialog_id, creator).unwrap());
        assert!(!t.db.is_dialog_admin(dialog_id, member).unwrap());

        t.db.remove_participant(dialog_id, member).unwrap();
        let meta = t.db.get_dialog_meta(dialog_id, creator).unwrap().unwrap();
        assert_eq!(meta.participants, vec![creator]);
        // Даже с одним участником группа остаётся группой.
        assert!(meta.is_group);
    }

    #[test]
    fn user_chats_resolve_title_and_last_message() {
        let t = TempDb::new();
        let u1 = t.db.create_user("+2006", "pass").unwrap();
        let u2 = t.db.create_user("+2007", "pass").unwrap();
        let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();
        t.db.save_message(dialog_id, u2, "привет", 100, false, None, None)
            .unwrap();
        t.db.save_message(dialog_id, u1, "ответ", 200, true, None, None)
            .unwrap();

        let chats = t.db.get_user_chats(u1).unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].id, dialog_id);
        assert!(!chats[0].is_group);
        assert_eq!(chats[0].title, "+2007", "личный диалог называется по телефону");
        assert_eq!(chats[0].last_message.as_deref(), Some("ответ"));
        assert_eq!(chats[0].last_timestamp, Some(200));

        let group_id = t.db.create_group_dialog("Друзья", u1, &[u2]).unwrap();
        let chats = t.db.get_user_chats(u1).unwrap();
        let group = chats.iter().find(|c| c.id == group_id).expect("группа в списке");
        assert!(group.is_group);
        assert_eq!(group.title, "Друзья");
    }

    #[test]
    fn stale_sessions_are_purged() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+2008", "pass").unwrap();
        save_test_session(&t.db, 1, user_id);
        save_test_session(&t.db, 2, 0); // брошенное рукопожатие без SignUp

        // Пользовательская сессия не использовалась 90 000 секунд (> суток),
        // анонимное рукопожатие — 2 часа (> часа).
        t.db.get_conn()
            .unwrap()
            .execute(
                "UPDATE sessions SET last_used_at = CASE auth_key_id
                     WHEN 1 THEN ?1
                     ELSE ?2
                 END",
                rusqlite::params![now_secs() - 90_000, now_secs() - 7200],
            )
            .unwrap();

        // user_id = 0 чистятся через час, пользовательские сессии — через сутки.
        let removed = t.db.purge_stale_sessions(3600, 86_400).unwrap();
        assert_eq!(removed, 2);
        assert!(t.db.get_full_session(1).unwrap().is_none());
        assert!(t.db.get_full_session(2).unwrap().is_none());

        // Свежая анонимная сессия суточную очистку переживает.
        save_test_session(&t.db, 3, 0);
        let removed = t.db.purge_stale_sessions(3600, 86_400).unwrap();
        assert_eq!(removed, 0, "сессия, использованная только что, не удаляется");
    }

    #[test]
    fn duplicate_registration_is_an_error_not_a_panic() {
        let t = TempDb::new();
        let id = t.db.create_user("+2009", "pass").unwrap();
        // Гонка/повтор: второй INSERT OR IGNORE не должен давать 500 из-за UNIQUE.
        let err = t.db.create_user("+2009", "other").unwrap_err();
        assert!(format!("{}", err).contains("already registered"));
        assert_eq!(t.db.get_user_id_by_phone("+2009").unwrap(), Some(id));
    }

    #[test]
    fn migration_adds_new_columns_to_old_database() {
        let path = std::env::temp_dir().join(format!(
            "kosta_migration_test_{}.db",
            rand::random::<u64>()
        ));
        // Старая схема: без is_admin и last_used_at.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                r"
                CREATE TABLE users (id INTEGER PRIMARY KEY, phone TEXT UNIQUE NOT NULL,
                                    password_hash TEXT NOT NULL, status INTEGER NOT NULL DEFAULT 0,
                                    last_seen INTEGER);
                CREATE TABLE sessions (auth_key_id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL,
                    server_salt INTEGER NOT NULL, nonce BLOB NOT NULL, server_nonce BLOB NOT NULL,
                    new_nonce BLOB NOT NULL, recv_seq_no INTEGER NOT NULL DEFAULT -1,
                    auth_key BLOB NOT NULL, client_write_key BLOB NOT NULL,
                    client_mac_key BLOB NOT NULL, server_write_key BLOB NOT NULL,
                    server_mac_key BLOB NOT NULL, created_at INTEGER NOT NULL,
                    last_msg_id INTEGER NOT NULL DEFAULT 0, send_counter INTEGER NOT NULL DEFAULT 0,
                    recv_counter INTEGER NOT NULL DEFAULT 0, send_seq_no INTEGER NOT NULL DEFAULT -1,
                    msg_id_counter INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE dialogs (id INTEGER PRIMARY KEY, type INTEGER NOT NULL,
                    title TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL);
                CREATE TABLE dialog_participants (dialog_id INTEGER, user_id INTEGER,
                    joined_at INTEGER NOT NULL, PRIMARY KEY (dialog_id, user_id));
                CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, dialog_id INTEGER NOT NULL,
                    sender_id INTEGER NOT NULL, text TEXT NOT NULL, timestamp INTEGER NOT NULL,
                    is_outgoing INTEGER NOT NULL, read_status INTEGER NOT NULL DEFAULT 0,
                    delivered_status INTEGER NOT NULL DEFAULT 0);
                INSERT INTO users (id, phone, password_hash) VALUES (0, 'system', '');
                INSERT INTO users (id, phone, password_hash) VALUES (1, '+3000', 'x');
                INSERT INTO users (id, phone, password_hash) VALUES (2, '+3001', 'x');
                INSERT INTO dialogs (id, type, title, created_at) VALUES (10, 1, 'Old group', 0);
                INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (10, 1, 100);
                INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (10, 2, 200);
                INSERT INTO sessions (auth_key_id, user_id, server_salt, nonce, server_nonce,
                    new_nonce, auth_key, client_write_key, client_mac_key, server_write_key,
                    server_mac_key, created_at) VALUES (7, 1, 0, zeroblob(16), zeroblob(16),
                    zeroblob(32), zeroblob(256), zeroblob(32), zeroblob(32), zeroblob(32),
                    zeroblob(32), 123);
                ",
            )
            .unwrap();
        }

        let secrets = Arc::new(SecretsBox::from_key([0x42u8; 32]));
        let db = Database::new(path.to_str().unwrap(), secrets).expect("миграция старой БД");
        // Первый вступивший в старую группу становится админом.
        assert!(db.is_dialog_admin(10, 1).unwrap());
        assert!(!db.is_dialog_admin(10, 2).unwrap());
        // last_used_at заполнен из created_at (123), поэтому старая сессия
        // не должна остаться с нулём и переживает миграцию как «не использованная
        // только что» — но суточная очистка её всё равно удалит.
        let last_used: i64 = db
            .get_conn()
            .unwrap()
            .query_row(
                "SELECT last_used_at FROM sessions WHERE auth_key_id = 7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_used, 123, "last_used_at должен быть заполнен из created_at");
        let removed = db.purge_stale_sessions(3600, 30 * 86_400).unwrap();
        assert_eq!(removed, 1, "очень старая сессия удаляется очисткой");
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn unread_count_counts_only_incoming_undeleted_unread() {
        let t = TempDb::new();
        let u1 = t.db.create_user("+2100", "pass").unwrap();
        let u2 = t.db.create_user("+2101", "pass").unwrap();
        let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();

        // Три входящих, одно своё и одно входящее «удалённое».
        let already_read = t
            .db
            .save_message(dialog_id, u2, "прочитано", 100, false, None, None)
            .unwrap();
        t.db
            .save_message(dialog_id, u2, "новое 1", 101, false, None, None)
            .unwrap();
        t.db
            .save_message(dialog_id, u2, "новое 2", 102, false, None, None)
            .unwrap();
        t.db
            .save_message(dialog_id, u1, "моё", 103, true, None, None)
            .unwrap();
        let deleted = t
            .db
            .save_message(dialog_id, u2, "удалено", 104, false, None, None)
            .unwrap();

        // Прочитанность и удаление — руками: публичных методов для них нет.
        let conn = t.db.get_conn().unwrap();
        conn.execute(
            "UPDATE messages SET read_status = 1 WHERE id = ?1",
            rusqlite::params![already_read],
        )
        .unwrap();
        conn.execute(
            "UPDATE messages SET deleted = 1 WHERE id = ?1",
            rusqlite::params![deleted],
        )
        .unwrap();
        drop(conn);

        let row = t.db.get_chat_row(u1, dialog_id).unwrap().unwrap();
        assert_eq!(row.unread_count, 2, "только два входящих непрочитанных");
        assert_eq!(row.other_user_id, Some(u2));
        assert_eq!(row.title, "+2101");
        assert!(!row.is_online, "без строки user_status собеседник offline");

        t.db.set_user_status(u2, 0).unwrap();
        assert!(
            t.db.get_chat_row(u1, dialog_id).unwrap().unwrap().is_online,
            "status 0 — онлайн"
        );
        t.db.set_user_status(u2, 1).unwrap();
        assert!(
            !t.db.get_chat_row(u1, dialog_id).unwrap().unwrap().is_online,
            "status 1 — офлайн"
        );

        // Для второго участника собеседник — первый.
        let peer = t.db.get_chat_row(u2, dialog_id).unwrap().unwrap();
        assert_eq!(peer.other_user_id, Some(u1));
        assert_eq!(peer.unread_count, 1, "непрочитанное входящее от u1");

        // У группы нет «собеседника» и статуса.
        let group_id = t.db.create_group_dialog("Группа", u1, &[u2]).unwrap();
        let group = t.db.get_chat_row(u1, group_id).unwrap().unwrap();
        assert!(group.is_group);
        assert_eq!(group.other_user_id, None);
        assert!(!group.is_online);
        assert_eq!(group.unread_count, 0);

        // Постороннему карточка не положена.
        let stranger = t.db.create_user("+2102", "pass").unwrap();
        assert!(t.db.get_chat_row(stranger, dialog_id).unwrap().is_none());
        assert!(t.db.get_chat_row(stranger, group_id).unwrap().is_none());
    }

    #[test]
    fn mark_dialog_read_marks_only_incoming_and_is_idempotent() {
        let t = TempDb::new();
        let u1 = t.db.create_user("+2103", "pass").unwrap();
        let u2 = t.db.create_user("+2104", "pass").unwrap();
        let dialog_id = t.db.create_private_dialog(u1, u2).unwrap();

        t.db
            .save_message(dialog_id, u2, "входящее 1", 100, false, None, None)
            .unwrap();
        t.db
            .save_message(dialog_id, u2, "входящее 2", 101, false, None, None)
            .unwrap();
        t.db
            .save_message(dialog_id, u1, "исходящее", 102, true, None, None)
            .unwrap();

        // Для u1 входящих непрочитанных два, исходящее не трогаем.
        assert_eq!(t.db.mark_dialog_read(dialog_id, u1).unwrap(), 2);
        assert_eq!(
            t.db.mark_dialog_read(dialog_id, u1).unwrap(),
            0,
            "повторный вызов нечего помечать"
        );
        assert_eq!(
            t.db.get_chat_row(u1, dialog_id).unwrap().unwrap().unread_count,
            0
        );

        // Для u2 входящее — одно (сообщение u1), остальное его собственное.
        assert_eq!(t.db.mark_dialog_read(dialog_id, u2).unwrap(), 1);
        assert_eq!(t.db.mark_dialog_read(dialog_id, u2).unwrap(), 0);
    }

    #[test]
    fn detailed_participants_include_phone_role_and_online() {
        let t = TempDb::new();
        let creator = t.db.create_user("+2105", "pass").unwrap();
        let member = t.db.create_user("+2106", "pass").unwrap();
        let offline = t.db.create_user("+2107", "pass").unwrap();
        let group_id = t
            .db
            .create_group_dialog("Команда", creator, &[member, offline])
            .unwrap();
        t.db.set_user_status(creator, 1).unwrap();
        t.db.set_user_status(member, 0).unwrap();

        let participants = t.db.get_dialog_participants_detailed(group_id).unwrap();
        assert_eq!(participants.len(), 3);
        let by_id = |id: i64| {
            participants
                .iter()
                .find(|p| p.user_id == id)
                .expect("участник должен быть в списке")
        };
        assert!(by_id(creator).is_admin, "создатель — администратор");
        assert_eq!(by_id(creator).phone, "+2105");
        assert!(!by_id(creator).is_online, "status 1 — офлайн");
        assert!(!by_id(member).is_admin);
        assert!(by_id(member).is_online, "status 0 — онлайн");
        assert!(
            !by_id(offline).is_online,
            "отсутствие строки user_status — offline"
        );

        // Личный диалог тоже отдаёт участников (для карточки чата).
        let private_id = t.db.create_private_dialog(member, offline).unwrap();
        let private = t.db.get_dialog_participants_detailed(private_id).unwrap();
        assert_eq!(private.len(), 2);
        assert!(private.iter().all(|p| !p.is_admin));
    }

    #[test]
    fn rename_dialog_updates_title_and_reports_missing_dialog() {
        let t = TempDb::new();
        let creator = t.db.create_user("+2109", "pass").unwrap();
        let group_id = t.db.create_group_dialog("Старое", creator, &[]).unwrap();

        t.db.rename_dialog(group_id, "Новое").unwrap();
        let meta = t.db.get_dialog_meta(group_id, creator).unwrap().unwrap();
        assert_eq!(meta.title, "Новое");
        assert!(t.db.rename_dialog(999_999, "x").is_err());
    }

    #[test]
    fn lookup_by_phone_is_exact_and_profile_reports_online() {
        let t = TempDb::new();
        let user_id = t.db.create_user("+2110", "pass").unwrap();

        assert_eq!(
            t.db.find_user_by_phone("+2110").unwrap(),
            Some((user_id, "+2110".to_string()))
        );
        assert_eq!(t.db.find_user_by_phone("+2111").unwrap(), None);
        assert_eq!(
            t.db.find_user_by_phone("2110").unwrap(),
            None,
            "номер сверяется точно, без подстрок"
        );

        assert_eq!(
            t.db.get_user_profile(user_id).unwrap(),
            Some(("+2110".to_string(), false)),
            "нет строки user_status — offline"
        );
        t.db.set_user_status(user_id, 0).unwrap();
        assert_eq!(
            t.db.get_user_profile(user_id).unwrap(),
            Some(("+2110".to_string(), true))
        );
        t.db.set_user_status(user_id, 3).unwrap();
        assert_eq!(
            t.db.get_user_profile(user_id).unwrap(),
            Some(("+2110".to_string(), false)),
            "любой статус кроме 0 — offline"
        );
        assert!(t.db.get_user_profile(999_999).unwrap().is_none());
    }
}
