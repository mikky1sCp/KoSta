// kosta-server/src/db.rs
use anyhow::{anyhow, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
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

/// Краткие данные сессии для проверки авторизации.
pub type SessionLookup = (i64, i64, Vec<u8>, Vec<u8>, Vec<u8>);

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
        let manager = SqliteConnectionManager::file(path);
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

        // ---- МИГРАЦИЯ: добавляем новые колонки в messages ----
        let columns = [
            ("media_path", "TEXT"),
            ("media_type", "TEXT"),
            ("edited_at", "INTEGER"),
            ("deleted", "INTEGER NOT NULL DEFAULT 0"),
        ];
        for (name, typ) in columns {
            // Проверяем наличие колонки через PRAGMA table_info
            let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
            let mut exists = false;
            let rows = stmt.query_map([], |row| {
                let col_name: String = row.get(1)?;
                Ok(col_name)
            })?;
            for col_name in rows {
                if col_name? == name {
                    exists = true;
                    break;
                }
            }
            if !exists {
                conn.execute(&format!("ALTER TABLE messages ADD COLUMN {} {}", name, typ), [])?;
            }
        }

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
        conn.execute(
            "INSERT INTO users (phone, password_hash) VALUES (?1, ?2)",
            rusqlite::params![phone, hashed],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn authenticate(&self, phone: &str, password: &str) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT id, password_hash FROM users WHERE phone = ?1")?;
        let mut rows = stmt.query(rusqlite::params![phone])?;
        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let hash: String = row.get(1)?;
            let valid = bcrypt::verify(password, &hash)
                .map_err(|e| anyhow!("bcrypt verify error: {}", e))?;
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
              last_msg_id, send_counter, recv_counter, send_seq_no, msg_id_counter)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now'),
                     ?13, ?14, ?15, ?16, ?17)",
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
        if let Some(row) = rows.next()? {
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

            let mut nonce_arr = [0u8; 16];
            nonce_arr.copy_from_slice(&nonce);
            let mut server_nonce_arr = [0u8; 16];
            server_nonce_arr.copy_from_slice(&server_nonce);
            let mut new_nonce_arr = [0u8; 32];
            new_nonce_arr.copy_from_slice(&new_nonce);
            // Секреты расшифровываются мастер-ключом; легаси-записи читаются как есть.
            let auth_key_arr: [u8; 256] = self.secrets.open_fixed(&auth_key)?;
            let cwk: [u8; 32] = self.secrets.open_fixed(&client_write_key)?;
            let cmk: [u8; 32] = self.secrets.open_fixed(&client_mac_key)?;
            let swk: [u8; 32] = self.secrets.open_fixed(&server_write_key)?;
            let smk: [u8; 32] = self.secrets.open_fixed(&server_mac_key)?;

            Ok(Some(FullSession {
                auth_key_id,
                user_id,
                server_salt,
                nonce: nonce_arr,
                server_nonce: server_nonce_arr,
                new_nonce: new_nonce_arr,
                recv_seq_no,
                auth_key: auth_key_arr,
                client_write_key: cwk,
                client_mac_key: cmk,
                server_write_key: swk,
                server_mac_key: smk,
                last_msg_id,
                send_counter: send_counter as u32,
                recv_counter: recv_counter as u32,
                send_seq_no,
                msg_id_counter: msg_id_counter as u32,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_full_session_for_user(&self, user_id: i64) -> Result<Option<FullSession>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT auth_key_id, user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
                    auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key,
                    last_msg_id, send_counter, recv_counter, send_seq_no, msg_id_counter
             FROM sessions WHERE user_id = ?1 ORDER BY created_at DESC LIMIT 1"
        )?;
        let mut rows = stmt.query(rusqlite::params![user_id])?;
        if let Some(row) = rows.next()? {
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

            let mut nonce_arr = [0u8; 16];
            nonce_arr.copy_from_slice(&nonce);
            let mut server_nonce_arr = [0u8; 16];
            server_nonce_arr.copy_from_slice(&server_nonce);
            let mut new_nonce_arr = [0u8; 32];
            new_nonce_arr.copy_from_slice(&new_nonce);
            // Секреты расшифровываются мастер-ключом; легаси-записи читаются как есть.
            let auth_key_arr: [u8; 256] = self.secrets.open_fixed(&auth_key)?;
            let cwk: [u8; 32] = self.secrets.open_fixed(&client_write_key)?;
            let cmk: [u8; 32] = self.secrets.open_fixed(&client_mac_key)?;
            let swk: [u8; 32] = self.secrets.open_fixed(&server_write_key)?;
            let smk: [u8; 32] = self.secrets.open_fixed(&server_mac_key)?;

            Ok(Some(FullSession {
                auth_key_id,
                user_id,
                server_salt,
                nonce: nonce_arr,
                server_nonce: server_nonce_arr,
                new_nonce: new_nonce_arr,
                recv_seq_no,
                auth_key: auth_key_arr,
                client_write_key: cwk,
                client_mac_key: cmk,
                server_write_key: swk,
                server_mac_key: smk,
                last_msg_id,
                send_counter: send_counter as u32,
                recv_counter: recv_counter as u32,
                send_seq_no,
                msg_id_counter: msg_id_counter as u32,
            }))
        } else {
            Ok(None)
        }
    }

    // старый метод (оставлен для совместимости)
    pub fn get_session(&self, auth_key_id: i64) -> Result<Option<SessionLookup>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT user_id, server_salt, nonce, server_nonce, new_nonce FROM sessions WHERE auth_key_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![auth_key_id])?;
        if let Some(row) = rows.next()? {
            let user_id: i64 = row.get(0)?;
            let server_salt: i64 = row.get(1)?;
            let nonce: Vec<u8> = row.get(2)?;
            let server_nonce: Vec<u8> = row.get(3)?;
            let new_nonce: Vec<u8> = row.get(4)?;
            Ok(Some((user_id, server_salt, nonce, server_nonce, new_nonce)))
        } else {
            Ok(None)
        }
    }

    // ===== ДИАЛОГИ =====
    pub fn create_private_dialog(&self, user1: i64, user2: i64) -> Result<i64> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT d.id FROM dialogs d
             JOIN dialog_participants dp1 ON d.id = dp1.dialog_id
             JOIN dialog_participants dp2 ON d.id = dp2.dialog_id
             WHERE d.type = 0 AND dp1.user_id = ?1 AND dp2.user_id = ?2"
        )?;
        let mut rows = stmt.query(rusqlite::params![user1, user2])?;
        if let Some(row) = rows.next()? {
            return Ok(row.get(0)?);
        }
        conn.execute(
            "INSERT INTO dialogs (type, title, created_at) VALUES (0, '', strftime('%s','now'))",
            [],
        )?;
        let dialog_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (?1, ?2, strftime('%s','now'))",
            rusqlite::params![dialog_id, user1],
        )?;
        conn.execute(
            "INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (?1, ?2, strftime('%s','now'))",
            rusqlite::params![dialog_id, user2],
        )?;
        Ok(dialog_id)
    }

    pub fn create_group_dialog(&self, title: &str, creator: i64, participants: &[i64]) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO dialogs (type, title, created_at) VALUES (1, ?1, strftime('%s','now'))",
            rusqlite::params![title],
        )?;
        let dialog_id = conn.last_insert_rowid();
        let mut all_participants = vec![creator];
        all_participants.extend_from_slice(participants);
        for user_id in all_participants {
            conn.execute(
                "INSERT INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (?1, ?2, strftime('%s','now'))",
                rusqlite::params![dialog_id, user_id],
            )?;
        }
        Ok(dialog_id)
    }

    pub fn get_user_dialogs(&self, user_id: i64) -> Result<Vec<(i64, String, i64, String)>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT d.id, d.title, d.type,
                    (SELECT u.phone FROM users u
                     JOIN dialog_participants dp ON dp.user_id = u.id
                     WHERE dp.dialog_id = d.id AND u.id != ?1 LIMIT 1) as other_phone
             FROM dialogs d
             JOIN dialog_participants dp ON d.id = dp.dialog_id
             WHERE dp.user_id = ?1
             ORDER BY (SELECT MAX(timestamp) FROM messages WHERE dialog_id = d.id) DESC"
        )?;
        let rows = stmt.query_map(rusqlite::params![user_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, Option<String>>(3)?.unwrap_or_default()))
        })?;
        let mut dialogs = Vec::new();
        for row in rows {
            dialogs.push(row?);
        }
        Ok(dialogs)
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

    pub fn add_participant(&self, dialog_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO dialog_participants (dialog_id, user_id, joined_at) VALUES (?1, ?2, strftime('%s','now'))",
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

        // В файле БД не должно быть ключей открытым текстом
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
    }
}