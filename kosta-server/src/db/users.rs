// kosta-server/src/db/users.rs
//! Пользователи: регистрация, аутентификация, профиль и статус.

use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;

use super::{Database, UserProfileRow, UserSearchRow};

impl Database {
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

    /// Меняет отображаемое имя. Пустая строка сбрасывает имя (останется телефон).
    pub fn update_display_name(&self, user_id: i64, display_name: &str) -> Result<()> {
        let trimmed = display_name.trim();
        let value: Option<&str> = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        let lower: Option<String> = value.map(|name| name.to_lowercase());
        let conn = self.get_conn()?;
        let updated = conn.execute(
            "UPDATE users SET display_name = ?1, display_name_lower = ?2 WHERE id = ?3",
            rusqlite::params![value, lower, user_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("пользователь {} не найден", user_id));
        }
        Ok(())
    }

    /// Ставит публичный `@username`. Пустая строка снимает username.
    ///
    /// Уникальность обеспечивает индекс `idx_users_username`; здесь заранее
    /// проверяем занятость, чтобы отдать понятную ошибку, а не 500 от SQLite.
    pub fn update_username(&self, user_id: i64, username: &str) -> Result<()> {
        let trimmed = username.trim().trim_start_matches('@');
        let value: Option<String> = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_lowercase())
        };
        if let Some(candidate) = value.as_deref() {
            if let Some(existing) = self.find_user_by_username(candidate)? {
                if existing != user_id {
                    return Err(anyhow!("username is already taken"));
                }
            }
        }
        let conn = self.get_conn()?;
        let updated = conn.execute(
            "UPDATE users SET username = ?1 WHERE id = ?2",
            rusqlite::params![value, user_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("пользователь {} не найден", user_id));
        }
        Ok(())
    }

    /// Точный поиск по `@username` (без учёта регистра).
    pub fn find_user_by_username(&self, username: &str) -> Result<Option<i64>> {
        let needle = username.trim().trim_start_matches('@').to_lowercase();
        let conn = self.get_conn()?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                rusqlite::params![needle],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found)
    }

    /// Поиск людей по `@username` (префикс) или отображаемому имени (подстрока).
    ///
    /// Возвращает только безопасные для показа поля — без телефона и хеша
    /// пароля. Системный пользователь (`id = 0`) и сам искатель исключены.
    pub fn search_users(
        &self,
        query: &str,
        limit: i32,
        exclude_user_id: i64,
    ) -> Result<Vec<UserSearchRow>> {
        let needle = query.trim().to_lowercase();
        let limit = limit.clamp(1, 50);
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.display_name, u.avatar_key,
                    COALESCE(us.status = 0, 0)
             FROM users u
             LEFT JOIN user_status us ON us.user_id = u.id
             WHERE u.id <> 0 AND u.id <> ?1
               AND (u.username LIKE ?2 ESCAPE '\\' OR u.display_name_lower LIKE ?3 ESCAPE '\\')
             ORDER BY u.username IS NULL, u.id
             LIMIT ?4",
        )?;
        let username_needle = format!("%{}%", Self::escape_like(&needle));
        let name_needle = format!("%{}%", Self::escape_like(&needle));
        let rows = stmt.query_map(
            rusqlite::params![exclude_user_id, username_needle, name_needle, limit],
            |row| {
                Ok(UserSearchRow {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    display_name: row.get(2)?,
                    avatar_key: row.get(3)?,
                    is_online: row.get::<_, i64>(4)? != 0,
                })
            },
        )?;
        let mut found = Vec::new();
        for row in rows {
            found.push(row?);
        }
        Ok(found)
    }

    /// Экранирует спецсимволы `LIKE` (`%`, `_` и сам `\`).
    fn escape_like(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());
        for ch in value.chars() {
            if matches!(ch, '\\' | '%' | '_') {
                escaped.push('\\');
            }
            escaped.push(ch);
        }
        escaped
    }

    /// Ставит ключ аватара и возвращает предыдущий (чтобы удалить файл).
    pub fn set_avatar_key(&self, user_id: i64, key: &str) -> Result<Option<String>> {
        let conn = self.get_conn()?;
        let previous: Option<String> = conn
            .query_row(
                "SELECT avatar_key FROM users WHERE id = ?1",
                rusqlite::params![user_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let updated = conn.execute(
            "UPDATE users SET avatar_key = ?1 WHERE id = ?2",
            rusqlite::params![key, user_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("пользователь {} не найден", user_id));
        }
        Ok(previous)
    }

    /// Ключ аватара пользователя (для отдачи картинки).
    pub fn get_avatar_key(&self, user_id: i64) -> Result<Option<String>> {
        let conn = self.get_conn()?;
        let key: Option<String> = conn
            .query_row(
                "SELECT avatar_key FROM users WHERE id = ?1",
                rusqlite::params![user_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(key)
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

    /// Профиль пользователя для `GET /api/me`: телефон, имя, аватар, статус.
    ///
    /// Статус берётся из `user_status`, а не из реестра WebSocket-соединений:
    /// нет строки — значит offline.
    pub fn get_user_profile(&self, user_id: i64) -> Result<Option<UserProfileRow>> {
        let conn = self.get_conn()?;
        let profile = conn
            .query_row(
                "SELECT u.phone, COALESCE(us.status = 0, 0), u.display_name, u.avatar_key, u.username
                 FROM users u
                 LEFT JOIN user_status us ON us.user_id = u.id
                 WHERE u.id = ?1",
                rusqlite::params![user_id],
                |row| {
                    Ok(UserProfileRow {
                        phone: row.get(0)?,
                        is_online: row.get::<_, i64>(1)? != 0,
                        display_name: row.get(2)?,
                        avatar_key: row.get(3)?,
                        username: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(profile)
    }

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
