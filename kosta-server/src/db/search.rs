// kosta-server/src/db/search.rs
//! Поиск по тексту сообщений.

use anyhow::Result;

use super::rows::history_row_from_row;
use super::{Database, HistoryRow};

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
