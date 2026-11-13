// kosta-server/src/db/mod.rs
//! Слой доступа к данным: пул SQLite, схема/миграции и операции по доменам.

use anyhow::Result;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use std::sync::Arc;

use crate::secrets::SecretsBox;

mod dialogs;
mod messages;
mod mtproto;
mod rows;
mod search;
mod tokens;
mod users;

#[cfg(test)]
mod tests;

pub use rows::{
    ChatRow, DialogMeta, GlobalSearchHit, HistoryRow, MediaMessageRow, MessageMeta,
    MtprotoSessionRow, ParticipantRow, ReactionRow, UserProfileRow, UserSearchRow,
};

pub struct Database {
    pool: Pool<SqliteConnectionManager>,
    /// Мастер-ключ для шифрования секретов сессий в БД.
    secrets: Arc<SecretsBox>,
}

/// Добавляет колонку, если её ещё нет (миграция старых баз).
fn ensure_column(conn: &rusqlite::Connection, table: &str, column: &str, decl: &str) -> Result<()> {
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

/// Заполняет `text_lower` у сообщений, созданных до появления колонки.
fn backfill_text_lower(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, text FROM messages WHERE text_lower IS NULL")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    if rows.is_empty() {
        return Ok(());
    }
    let mut update = conn.prepare("UPDATE messages SET text_lower = ?1 WHERE id = ?2")?;
    for (id, text) in rows {
        update.execute(rusqlite::params![text.to_lowercase(), id])?;
    }
    Ok(())
}

/// Заполняет `display_name_lower` у пользователей, созданных до появления
/// колонки. SQLite `lower()` не умеет кириллицу, поэтому нормализуем в Rust.
fn backfill_display_name_lower(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name FROM users WHERE display_name IS NOT NULL
         AND display_name_lower IS NULL",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    if rows.is_empty() {
        return Ok(());
    }
    let mut update = conn.prepare("UPDATE users SET display_name_lower = ?1 WHERE id = ?2")?;
    for (id, name) in rows {
        update.execute(rusqlite::params![name.to_lowercase(), id])?;
    }
    Ok(())
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
            CREATE TABLE IF NOT EXISTS mtproto_sessions (
                auth_key_id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL DEFAULT 0,
                salt INTEGER NOT NULL,
                auth_key BLOB NOT NULL,
                last_msg_id INTEGER NOT NULL DEFAULT 0,
                recv_seq_no INTEGER NOT NULL DEFAULT -1,
                send_seq_no INTEGER NOT NULL DEFAULT -1,
                msg_id_counter INTEGER NOT NULL DEFAULT 0,
                protocol_version INTEGER NOT NULL DEFAULT 2,
                created_at INTEGER NOT NULL,
                last_used_at INTEGER NOT NULL
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
            CREATE TABLE IF NOT EXISTS user_status (                user_id INTEGER PRIMARY KEY,
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
            CREATE TABLE IF NOT EXISTS message_reactions (
                message_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                emoji TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY(message_id, user_id, emoji),
                FOREIGN KEY(message_id) REFERENCES messages(id),
                FOREIGN KEY(user_id) REFERENCES users(id)
            );
            ",
        )?;

        // Индексы
        conn.execute_batch(
            r"
            CREATE INDEX IF NOT EXISTS idx_messages_dialog_id ON messages(dialog_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_mtproto_sessions_user_id ON mtproto_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_mtproto_sessions_last_used ON mtproto_sessions(last_used_at);
            CREATE INDEX IF NOT EXISTS idx_dialog_participants_user_id ON dialog_participants(user_id);
            CREATE INDEX IF NOT EXISTS idx_auth_tokens_user_id ON auth_tokens(user_id);
            CREATE INDEX IF NOT EXISTS idx_auth_tokens_expires_at ON auth_tokens(expires_at);
            CREATE INDEX IF NOT EXISTS idx_message_reactions_message ON message_reactions(message_id);
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
        ensure_column(&conn, "messages", "media_size", "INTEGER")?;
        ensure_column(&conn, "messages", "media_name", "TEXT")?;
        ensure_column(&conn, "messages", "edited_at", "INTEGER")?;
        ensure_column(&conn, "messages", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
        // Ответ на сообщение (цитата). NULL — обычное сообщение.
        ensure_column(&conn, "messages", "reply_to_id", "INTEGER")?;
        // Нормализованный текст для поиска: SQLite LIKE не умеет регистро-
        // независимый поиск для кириллицы, поэтому lower() считает Rust.
        ensure_column(&conn, "messages", "text_lower", "TEXT")?;
        backfill_text_lower(&conn)?;
        // dialog_participants: роли (создатель группы — администратор)
        ensure_column(
            &conn,
            "dialog_participants",
            "is_admin",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        // users: отображаемое имя и ключ аватара в media-хранилище.
        ensure_column(&conn, "users", "display_name", "TEXT")?;
        ensure_column(&conn, "users", "avatar_key", "TEXT")?;
        // Публичный @username и нормализованное имя для поиска людей.
        // Уникальный индекс допускает много NULL (SQLite), поэтому username
        // необязателен, а дубликаты ловятся на уровне БД.
        ensure_column(&conn, "users", "username", "TEXT")?;
        ensure_column(&conn, "users", "display_name_lower", "TEXT")?;
        backfill_display_name_lower(&conn)?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username)",
            [],
        )?;
        // dialogs: закреплённое сообщение (одно на диалог).
        ensure_column(&conn, "dialogs", "pinned_message_id", "INTEGER")?;
        // dialog_participants: личное состояние чата у каждого участника —
        // закрепление, мьют, архив, удаление из списка и ручная пометка
        // «непрочитано». Общее состояние диалога (название, pinned_message_id)
        // живёт в `dialogs`, а всё персональное — здесь.
        ensure_column(&conn, "dialog_participants", "pinned_at", "INTEGER")?;
        // NULL — уведомления включены; 0 — навсегда; >0 — до этого момента.
        ensure_column(&conn, "dialog_participants", "muted_until", "INTEGER")?;
        ensure_column(
            &conn,
            "dialog_participants",
            "archived",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "dialog_participants",
            "deleted",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "dialog_participants",
            "marked_unread",
            "INTEGER NOT NULL DEFAULT 0",
        )?;

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

        Ok(())
    }

    pub fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        Ok(self.pool.get()?)
    }
}

/// Текущее время в unix-секундах.
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
