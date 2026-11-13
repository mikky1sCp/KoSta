// kosta-server/src/db/mtproto.rs
//! Сохранение и загрузка сессий MTProto 2.0.

use anyhow::Result;
use rusqlite::OptionalExtension;

use super::{now_secs, Database, MtprotoSessionRow};

impl Database {
    // ----- Сессии MTProto 2.0 -----

    /// Сохраняет (или перезаписывает) сессию MTProto 2.0. `auth_key`
    /// в БД шифруется мастер-ключом, как и легаси-секреты.
    pub fn save_mtproto_session(&self, row: &MtprotoSessionRow) -> Result<()> {
        let sealed_auth_key = self.secrets.seal(&row.auth_key)?;
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO mtproto_sessions
                (auth_key_id, user_id, salt, auth_key, last_msg_id, recv_seq_no,
                 send_seq_no, msg_id_counter, protocol_version, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 2,
                     strftime('%s','now'), strftime('%s','now'))
             ON CONFLICT(auth_key_id) DO UPDATE SET
                user_id = excluded.user_id,
                salt = excluded.salt,
                auth_key = excluded.auth_key,
                last_msg_id = excluded.last_msg_id,
                recv_seq_no = excluded.recv_seq_no,
                send_seq_no = excluded.send_seq_no,
                msg_id_counter = excluded.msg_id_counter,
                last_used_at = strftime('%s','now')",
            rusqlite::params![
                row.auth_key_id,
                row.user_id,
                row.salt,
                sealed_auth_key,
                row.last_msg_id,
                row.recv_seq_no,
                row.send_seq_no,
                row.msg_id_counter,
            ],
        )?;
        Ok(())
    }

    /// Загружает сессию MTProto 2.0 по `auth_key_id`.
    pub fn load_mtproto_session(&self, auth_key_id: i64) -> Result<Option<MtprotoSessionRow>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT auth_key_id, user_id, salt, auth_key, last_msg_id, recv_seq_no,
                    send_seq_no, msg_id_counter
             FROM mtproto_sessions WHERE auth_key_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![auth_key_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let sealed: Vec<u8> = row.get(3)?;
        let auth_key = self.secrets.open_fixed::<256>(&sealed)?;
        Ok(Some(MtprotoSessionRow {
            auth_key_id: row.get(0)?,
            user_id: row.get(1)?,
            salt: row.get(2)?,
            auth_key,
            last_msg_id: row.get(4)?,
            recv_seq_no: row.get(5)?,
            send_seq_no: row.get(6)?,
            msg_id_counter: row.get(7)?,
        }))
    }

    /// `auth_key_id` последней MTProto 2.0 сессии пользователя (для web-справки).
    pub fn get_mtproto_auth_key_id_for_user(&self, user_id: i64) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let auth_key_id = conn
            .query_row(
                "SELECT auth_key_id FROM mtproto_sessions
                 WHERE user_id = ?1 ORDER BY last_used_at DESC LIMIT 1",
                rusqlite::params![user_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(auth_key_id)
    }

    /// Привязывает сессию к пользователю после `signUp`.
    pub fn update_mtproto_session_user_id(&self, auth_key_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE mtproto_sessions SET user_id = ?1, last_used_at = strftime('%s','now')
             WHERE auth_key_id = ?2",
            rusqlite::params![user_id, auth_key_id],
        )?;
        Ok(())
    }

    /// Сохраняет счётчики `msg_id`/`seq_no` (вызывается не на каждый кадр).
    pub fn save_mtproto_session_state(
        &self,
        auth_key_id: i64,
        last_msg_id: i64,
        recv_seq_no: i32,
        send_seq_no: i32,
        msg_id_counter: u32,
    ) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE mtproto_sessions
             SET last_msg_id = ?1, recv_seq_no = ?2, send_seq_no = ?3,
                 msg_id_counter = ?4, last_used_at = strftime('%s','now')
             WHERE auth_key_id = ?5",
            rusqlite::params![
                last_msg_id,
                recv_seq_no,
                send_seq_no,
                msg_id_counter,
                auth_key_id
            ],
        )?;
        Ok(())
    }

    /// Удаляет брошенные сессии MTProto 2.0 (анонимные и давно неиспользуемые).
    pub fn purge_stale_mtproto_sessions(
        &self,
        anonymous_idle_secs: i64,
        idle_secs: i64,
    ) -> Result<usize> {
        let conn = self.get_conn()?;
        let now = now_secs();
        let removed = conn.execute(
            "DELETE FROM mtproto_sessions
             WHERE (user_id = 0 AND last_used_at < ?1)
                OR last_used_at < ?2",
            rusqlite::params![now - anonymous_idle_secs, now - idle_secs],
        )?;
        Ok(removed)
    }
}
