// kosta-server/src/db/search.rs
//! Поиск по тексту сообщений.

use anyhow::Result;

use super::rows::{global_search_row_from_row, history_row_from_row};
use super::{Database, GlobalSearchHit, HistoryRow};

impl Database {
    /// Поиск по тексту сообщений диалога без учёта регистра.
    ///
    /// Сравнение идёт по `text_lower`: `LIKE` в SQLite регистронезависим только
    /// для ASCII, поэтому кириллицу нормализует Rust при записи.
    pub fn search_messages(
        &self,
        dialog_id: i64,
        query: &str,
        limit: i32,
    ) -> Result<Vec<HistoryRow>> {
        let limit = limit.clamp(1, 200);
        let needle = format!("%{}%", escape_like(&query.to_lowercase()));
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.sender_id, m.text, m.timestamp, m.is_outgoing, m.read_status,
                    m.delivered_status, m.media_path, m.media_type, m.media_size, m.media_name,
                    m.reply_to_id, m.edited_at,
                    CASE WHEN r.deleted = 1 THEN NULL ELSE r.text END,
                    CASE WHEN r.deleted = 1 THEN NULL ELSE r.sender_id END,
                    r.media_path
             FROM messages m
             LEFT JOIN messages r ON r.id = m.reply_to_id
             WHERE m.dialog_id = ?1 AND m.deleted = 0
               AND m.text_lower LIKE ?2 ESCAPE '\\'
             ORDER BY m.timestamp DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![dialog_id, needle, limit],
            history_row_from_row,
        )?;
        let mut found = Vec::new();
        for row in rows {
            found.push(row?);
        }
        Ok(found)
    }

    /// Глобальный поиск по тексту сообщений во всех диалогах пользователя.
    ///
    /// Возвращает сообщения вместе с диалогом (`dialog_id` и `title`), чтобы
    /// UI мог показать, куда перейти. Реакции досыпает web-слой по диалогам.
    pub fn search_messages_global(
        &self,
        user_id: i64,
        query: &str,
        limit: i32,
    ) -> Result<Vec<GlobalSearchHit>> {
        let limit = limit.clamp(1, 200);
        let needle = format!("%{}%", escape_like(&query.to_lowercase()));
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.sender_id, m.text, m.timestamp, m.is_outgoing, m.read_status,
                    m.delivered_status, m.media_path, m.media_type, m.media_size, m.media_name,
                    m.reply_to_id, m.edited_at,
                    CASE WHEN r.deleted = 1 THEN NULL ELSE r.text END,
                    CASE WHEN r.deleted = 1 THEN NULL ELSE r.sender_id END,
                    r.media_path,
                    m.dialog_id,
                    CASE WHEN d.type = 1 THEN d.title ELSE (
                        SELECT COALESCE(NULLIF(TRIM(u.display_name), ''), u.phone)
                        FROM dialog_participants dp2
                        JOIN users u ON u.id = dp2.user_id
                        WHERE dp2.dialog_id = d.id AND dp2.user_id != ?1
                        ORDER BY u.id LIMIT 1
                    ) END AS title
             FROM messages m
             JOIN dialogs d ON d.id = m.dialog_id
             JOIN dialog_participants dp ON dp.dialog_id = m.dialog_id AND dp.user_id = ?1
             LEFT JOIN messages r ON r.id = m.reply_to_id
             WHERE m.deleted = 0
               AND m.text_lower LIKE ?2 ESCAPE '\\'
             ORDER BY m.timestamp DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![user_id, needle, limit],
            global_search_row_from_row,
        )?;
        let mut found = Vec::new();
        for row in rows {
            found.push(row?);
        }
        Ok(found)
    }
}

/// Экранирует спецсимволы `LIKE` (`%`, `_` и сам `\`), чтобы запрос
/// «100%» не превращался в шаблон.
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
