// kosta-server/src/db.rs
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

// структура для полной сессии (без изменений)
#[derive(Debug, Clone)]
pub struct FullSession {
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
}

pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager)?;
        let db = Database { pool };
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
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS chats (
                id INTEGER PRIMARY KEY,
                title TEXT,
                is_group INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS chat_participants (
                chat_id INTEGER,
                user_id INTEGER,
                PRIMARY KEY (chat_id, user_id),
                FOREIGN KEY(chat_id) REFERENCES chats(id),
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                sender_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                is_outgoing INTEGER NOT NULL,
                read_status INTEGER NOT NULL DEFAULT 0,
                delivered_status INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(chat_id) REFERENCES chats(id),
                FOREIGN KEY(sender_id) REFERENCES users(id)
            );
            ",
        )?;

        conn.execute_batch(
            r"
            CREATE INDEX IF NOT EXISTS idx_messages_chat_id ON messages(chat_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_sender_id ON messages(sender_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
            "
        )?;

        // Добавляем системного пользователя
        conn.execute(
            "INSERT OR IGNORE INTO users (id, phone, password_hash) VALUES (0, 'system', '')",
            [],
        )?;

        // Добавляем тестовый чат
        conn.execute(
            "INSERT OR IGNORE INTO chats (id, title, is_group) VALUES (1, 'Test Chat', 0)",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO chat_participants (chat_id, user_id) VALUES (1, 0)",
            [],
        )?;

        // Проверяем наличие колонок (миграция)
        let mut stmt = conn.prepare("PRAGMA table_info(sessions)")?;
        let mut columns = Vec::new();
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for col in rows {
            columns.push(col?);
        }
        if !columns.contains(&"last_msg_id".to_string()) {
            conn.execute("ALTER TABLE sessions ADD COLUMN last_msg_id INTEGER NOT NULL DEFAULT 0", [])?;
        }
        if !columns.contains(&"send_counter".to_string()) {
            conn.execute("ALTER TABLE sessions ADD COLUMN send_counter INTEGER NOT NULL DEFAULT 0", [])?;
        }
        if !columns.contains(&"recv_counter".to_string()) {
            conn.execute("ALTER TABLE sessions ADD COLUMN recv_counter INTEGER NOT NULL DEFAULT 0", [])?;
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

    // ----- Сессии -----
    pub fn save_session_full(
        &self,
        auth_key_id: i64,
        user_id: i64,
        server_salt: i64,
        nonce: &[u8; 16],
        server_nonce: &[u8; 16],
        new_nonce: &[u8; 32],
        recv_seq_no: i32,
        auth_key: &[u8; 256],
        client_write_key: &[u8; 32],
        client_mac_key: &[u8; 32],
        server_write_key: &[u8; 32],
        server_mac_key: &[u8; 32],
        last_msg_id: i64,
        send_counter: u32,
        recv_counter: u32,
    ) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO sessions 
             (auth_key_id, user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
              auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key, created_at,
              last_msg_id, send_counter, recv_counter)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now'),
                     ?13, ?14, ?15)",
            rusqlite::params![
                auth_key_id, user_id, server_salt,
                nonce, server_nonce, new_nonce,
                recv_seq_no,
                auth_key,
                client_write_key, client_mac_key,
                server_write_key, server_mac_key,
                last_msg_id, send_counter, recv_counter,
            ],
        )?;
        Ok(())
    }

    pub fn update_session_recv_seq_and_last_msg(&self, auth_key_id: i64, recv_seq_no: i32, last_msg_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE sessions SET recv_seq_no = ?1, last_msg_id = ?2 WHERE auth_key_id = ?3",
            rusqlite::params![recv_seq_no, last_msg_id, auth_key_id],
        )?;
        Ok(())
    }

    pub fn update_session_counters(&self, auth_key_id: i64, send_counter: u32, recv_counter: u32) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE sessions SET send_counter = ?1, recv_counter = ?2 WHERE auth_key_id = ?3",
            rusqlite::params![send_counter, recv_counter, auth_key_id],
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
            "SELECT user_id, server_salt, nonce, server_nonce, new_nonce, recv_seq_no,
                    auth_key, client_write_key, client_mac_key, server_write_key, server_mac_key,
                    last_msg_id, send_counter, recv_counter
             FROM sessions WHERE auth_key_id = ?1"
        )?;
        let mut rows = stmt.query(rusqlite::params![auth_key_id])?;
        if let Some(row) = rows.next()? {
            let user_id: i64 = row.get(0)?;
            let server_salt: i64 = row.get(1)?;
            let nonce: Vec<u8> = row.get(2)?;
            let server_nonce: Vec<u8> = row.get(3)?;
            let new_nonce: Vec<u8> = row.get(4)?;
            let recv_seq_no: i32 = row.get(5)?;
            let auth_key: Vec<u8> = row.get(6)?;
            let client_write_key: Vec<u8> = row.get(7)?;
            let client_mac_key: Vec<u8> = row.get(8)?;
            let server_write_key: Vec<u8> = row.get(9)?;
            let server_mac_key: Vec<u8> = row.get(10)?;
            let last_msg_id: i64 = row.get(11)?;
            let send_counter: i64 = row.get(12)?;
            let recv_counter: i64 = row.get(13)?;

            let mut nonce_arr = [0u8; 16];
            nonce_arr.copy_from_slice(&nonce);
            let mut server_nonce_arr = [0u8; 16];
            server_nonce_arr.copy_from_slice(&server_nonce);
            let mut new_nonce_arr = [0u8; 32];
            new_nonce_arr.copy_from_slice(&new_nonce);
            let mut auth_key_arr = [0u8; 256];
            auth_key_arr.copy_from_slice(&auth_key);
            let mut cwk = [0u8; 32];
            cwk.copy_from_slice(&client_write_key);
            let mut cmk = [0u8; 32];
            cmk.copy_from_slice(&client_mac_key);
            let mut swk = [0u8; 32];
            swk.copy_from_slice(&server_write_key);
            let mut smk = [0u8; 32];
            smk.copy_from_slice(&server_mac_key);

            Ok(Some(FullSession {
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
            }))
        } else {
            Ok(None)
        }
    }

    // старый метод (оставлен для совместимости)
    pub fn get_session(&self, auth_key_id: i64) -> Result<Option<(i64, i64, Vec<u8>, Vec<u8>, Vec<u8>)>> {
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

    // ----- Чаты -----
    pub fn create_chat(&self, title: &str, is_group: bool, participants: &[i64]) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO chats (title, is_group) VALUES (?1, ?2)",
            rusqlite::params![title, if is_group { 1 } else { 0 }],
        )?;
        let chat_id = conn.last_insert_rowid();
        for &user_id in participants {
            conn.execute(
                "INSERT INTO chat_participants (chat_id, user_id) VALUES (?1, ?2)",
                rusqlite::params![chat_id, user_id],
            )?;
        }
        Ok(chat_id)
    }

    pub fn get_chat_participants(&self, chat_id: i64) -> Result<Vec<i64>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT user_id FROM chat_participants WHERE chat_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![chat_id], |row| row.get(0))?;
        let mut users = Vec::new();
        for user in rows {
            users.push(user?);
        }
        Ok(users)
    }

    // ----- Сообщения -----
    pub fn save_message(
        &self,
        chat_id: i64,
        sender_id: i64,
        text: &str,
        timestamp: i64,
        is_outgoing: bool,
    ) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO messages (chat_id, sender_id, text, timestamp, is_outgoing)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                chat_id,
                sender_id,
                text,
                timestamp,
                if is_outgoing { 1 } else { 0 }
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_history(&self, chat_id: i64, offset: i32, limit: i32) -> Result<Vec<(i64, i64, String, i64, bool, bool, bool)>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, sender_id, text, timestamp, is_outgoing, read_status, delivered_status
             FROM messages
             WHERE chat_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![chat_id, limit, offset],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get::<_, i32>(4)? != 0,
                    row.get::<_, i32>(5)? != 0,
                    row.get::<_, i32>(6)? != 0,
                ))
            },
        )?;
        let mut msgs = Vec::new();
        for msg in rows {
            msgs.push(msg?);
        }
        Ok(msgs)
    }

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

    // ----- Статусы пользователей -----
    pub fn set_user_status(&self, user_id: i64, status: i32) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE users SET status = ?1, last_seen = strftime('%s','now') WHERE id = ?2",
            rusqlite::params![status, user_id],
        )?;
        Ok(())
    }

    pub fn get_user_status(&self, user_id: i64) -> Result<Option<i32>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare("SELECT status FROM users WHERE id = ?1")?;
        let mut rows = stmt.query(rusqlite::params![user_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }
}