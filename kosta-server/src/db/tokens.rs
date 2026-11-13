// kosta-server/src/db/tokens.rs
//! Токены сессий web-API и WebSocket.

use anyhow::Result;

use super::{now_secs, Database};

impl Database {
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
