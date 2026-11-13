// kosta-server/src/db/messages.rs
//! Сообщения: история, доставка/прочтение, правка, удаление, реакции, медиа.

use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;
use std::collections::HashMap;

use super::rows::history_row_from_row;
use super::{Database, HistoryRow, MediaMessageRow, MessageMeta, ReactionRow};

impl Database {
    /// Одно сообщение по id (для закрепления и рассылки).
    pub fn get_message_row(&self, message_id: i64) -> Result<Option<HistoryRow>> {
        let conn = self.get_conn()?;
        let found = conn
            .query_row(
                "SELECT m.id, m.sender_id, m.text, m.timestamp, m.is_outgoing, m.read_status,
                        m.delivered_status, m.media_path, m.media_type, m.media_size, m.media_name,
                        m.reply_to_id, m.edited_at,
                        CASE WHEN r.deleted = 1 THEN NULL ELSE r.text END,
                        CASE WHEN r.deleted = 1 THEN NULL ELSE r.sender_id END,
                        r.media_path
                 FROM messages m
                 LEFT JOIN messages r ON r.id = m.reply_to_id
                 WHERE m.id = ?1 AND m.deleted = 0",
                rusqlite::params![message_id],
                history_row_from_row,
            )
            .optional()?;
        Ok(found)
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
        self.save_message_with_reply(
            dialog_id,
            sender_id,
            text,
            timestamp,
            is_outgoing,
            media_path,
            media_type,
            None,
        )
    }

    /// Как [`Self::save_message`], но с ответом на другое сообщение.
    #[allow(clippy::too_many_arguments)]
    pub fn save_message_with_reply(
        &self,
        dialog_id: i64,
        sender_id: i64,
        text: &str,
        timestamp: i64,
        is_outgoing: bool,
        media_path: Option<&str>,
        media_type: Option<&str>,
        reply_to_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO messages (dialog_id, sender_id, text, timestamp, is_outgoing, media_path, media_type, reply_to_id, text_lower)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                dialog_id,
                sender_id,
                text,
                timestamp,
                if is_outgoing { 1 } else { 0 },
                media_path,
                media_type,
                reply_to_id,
                text.to_lowercase(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Проверяет, что `message_id` — живое сообщение того же диалога.
    ///
    /// Нужно перед сохранением ответа: иначе можно ответить на сообщение
    /// из чужого диалога и вытащить его текст в цитате.
    pub fn is_replyable_in_dialog(&self, message_id: i64, dialog_id: i64) -> Result<bool> {
        let conn = self.get_conn()?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT id FROM messages WHERE id = ?1 AND dialog_id = ?2 AND deleted = 0",
                rusqlite::params![message_id, dialog_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Сохраняет сообщение с медиа-вложением и возвращает его id.
    ///
    /// Отдельно от [`Self::save_message`], потому что у медиа-сообщения есть
    /// обязательные метаданные (ключ хранения, MIME, размер, имя), и путать
    /// эти два пути нежелательно.
    #[allow(clippy::too_many_arguments)]
    pub fn save_media_message(
        &self,
        dialog_id: i64,
        sender_id: i64,
        text: &str,
        timestamp: i64,
        media_key: &str,
        mime: &str,
        size: i64,
        name: &str,
    ) -> Result<i64> {
        let conn = self.get_conn()?;
        conn.execute(
            "INSERT INTO messages
                (dialog_id, sender_id, text, timestamp, is_outgoing,
                 media_path, media_type, media_size, media_name)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8)",
            rusqlite::params![dialog_id, sender_id, text, timestamp, media_key, mime, size, name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Метаданные медиа-сообщения для скачивания.
    ///
    /// `None` — сообщения нет, оно удалено или это обычный текст.
    /// MIME у старых записей может быть пустым: отдаём безопасный fallback.
    pub fn get_media_message(&self, message_id: i64) -> Result<Option<MediaMessageRow>> {
        let conn = self.get_conn()?;
        let found = conn
            .query_row(
                "SELECT dialog_id, media_path, media_type, media_size, media_name
                 FROM messages
                 WHERE id = ?1 AND deleted = 0 AND media_path IS NOT NULL",
                rusqlite::params![message_id],
                |row| {
                    Ok(MediaMessageRow {
                        dialog_id: row.get(0)?,
                        media_key: row.get(1)?,
                        mime: row
                            .get::<_, Option<String>>(2)?
                            .unwrap_or_else(|| "application/octet-stream".to_string()),
                        size: row.get(3)?,
                        name: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(found)
    }

    pub fn get_history(&self, dialog_id: i64, offset: i32, limit: i32) -> Result<Vec<HistoryRow>> {
        // Защита в глубину: SQLite трактует отрицательный LIMIT как «без лимита»,
        // поэтому ограничение живёт ещё и здесь, а не только в хендлерах.
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
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
             ORDER BY m.timestamp DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![dialog_id, limit, offset],
            history_row_from_row,
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

    /// Меняет текст своего сообщения и возвращает время правки (unix seconds).
    ///
    /// Ошибка, если сообщение не найдено, уже удалено или принадлежит другому
    /// пользователю.
    pub fn edit_message(&self, message_id: i64, user_id: i64, new_text: &str) -> Result<i64> {
        if new_text.trim().is_empty() {
            return Err(anyhow!("Message text is empty"));
        }
        let conn = self.get_conn()?;
        // Проверяем, что пользователь является отправителем и сообщение не удалено
        let mut stmt =
            conn.prepare("SELECT sender_id FROM messages WHERE id = ?1 AND deleted = 0")?;
        let sender_id: i64 = stmt.query_row([message_id], |row| row.get(0))?;
        if sender_id != user_id {
            return Err(anyhow!("You are not the sender of this message"));
        }
        drop(stmt);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        conn.execute(
            "UPDATE messages SET text = ?1, edited_at = ?2, text_lower = ?3 WHERE id = ?4",
            rusqlite::params![new_text, now, new_text.to_lowercase(), message_id],
        )?;
        Ok(now)
    }

    /// Мягко удаляет своё сообщение (`deleted = 1`), история его больше не отдаёт.
    pub fn delete_message(&self, message_id: i64, user_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        // Проверяем, что пользователь является отправителем и сообщение не удалено
        let mut stmt =
            conn.prepare("SELECT sender_id FROM messages WHERE id = ?1 AND deleted = 0")?;
        let sender_id: i64 = stmt.query_row([message_id], |row| row.get(0))?;
        if sender_id != user_id {
            return Err(anyhow!("You are not the sender of this message"));
        }
        drop(stmt);
        conn.execute(
            "UPDATE messages SET deleted = 1 WHERE id = ?1",
            rusqlite::params![message_id],
        )?;
        Ok(())
    }

    // ===== РЕАКЦИИ =====

    /// Ставит реакцию. `false` — такая реакция у пользователя уже была.
    pub fn add_reaction(&self, message_id: i64, user_id: i64, emoji: &str) -> Result<bool> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let conn = self.get_conn()?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO message_reactions (message_id, user_id, emoji, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![message_id, user_id, emoji, now],
        )?;
        Ok(inserted > 0)
    }

    /// Снимает реакцию. `true` — реакция была и удалена.
    pub fn remove_reaction(&self, message_id: i64, user_id: i64, emoji: &str) -> Result<bool> {
        let conn = self.get_conn()?;
        let removed = conn.execute(
            "DELETE FROM message_reactions WHERE message_id = ?1 AND user_id = ?2 AND emoji = ?3",
            rusqlite::params![message_id, user_id, emoji],
        )?;
        Ok(removed > 0)
    }

    /// Агрегированные реакции одного сообщения.
    pub fn get_message_reactions(
        &self,
        message_id: i64,
        viewer_id: i64,
    ) -> Result<Vec<ReactionRow>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT emoji, COUNT(*), MAX(CASE WHEN user_id = ?2 THEN 1 ELSE 0 END)
             FROM message_reactions
             WHERE message_id = ?1
             GROUP BY emoji
             ORDER BY MIN(created_at), emoji",
        )?;
        let rows = stmt.query_map(rusqlite::params![message_id, viewer_id], |row| {
            Ok(ReactionRow {
                emoji: row.get(0)?,
                count: row.get(1)?,
                me: row.get::<_, i64>(2)? != 0,
            })
        })?;
        let mut reactions = Vec::new();
        for row in rows {
            reactions.push(row?);
        }
        Ok(reactions)
    }

    /// Реакции для всех сообщений диалога — одним запросом, без N+1.
    pub fn get_dialog_reactions(
        &self,
        dialog_id: i64,
        viewer_id: i64,
    ) -> Result<HashMap<i64, Vec<ReactionRow>>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.message_id, r.emoji, COUNT(*),
                    MAX(CASE WHEN r.user_id = ?2 THEN 1 ELSE 0 END)
             FROM message_reactions r
             JOIN messages m ON m.id = r.message_id
             WHERE m.dialog_id = ?1 AND m.deleted = 0
             GROUP BY r.message_id, r.emoji
             ORDER BY MIN(r.created_at), r.emoji",
        )?;
        let rows = stmt.query_map(rusqlite::params![dialog_id, viewer_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                ReactionRow {
                    emoji: row.get(1)?,
                    count: row.get(2)?,
                    me: row.get::<_, i64>(3)? != 0,
                },
            ))
        })?;
        let mut by_message: HashMap<i64, Vec<ReactionRow>> = HashMap::new();
        for row in rows {
            let (message_id, reaction) = row?;
            by_message.entry(message_id).or_default().push(reaction);
        }
        Ok(by_message)
    }

    /// Метаданные сообщения, нужные для рассылки правки/удаления.
    pub fn get_message_meta(&self, message_id: i64) -> Result<Option<MessageMeta>> {
        let conn = self.get_conn()?;
        let found = conn
            .query_row(
                "SELECT dialog_id, sender_id, text, media_path
                 FROM messages WHERE id = ?1 AND deleted = 0",
                rusqlite::params![message_id],
                |row| {
                    Ok(MessageMeta {
                        dialog_id: row.get(0)?,
                        sender_id: row.get(1)?,
                        text: row.get(2)?,
                        is_media: row.get::<_, Option<String>>(3)?.is_some(),
                    })
                },
            )
            .optional()?;
        Ok(found)
    }
}
