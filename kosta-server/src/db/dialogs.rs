// kosta-server/src/db/dialogs.rs
//! Диалоги, участники, роли и закреплённые сообщения.

use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;

use super::{ChatRow, Database, DialogMeta, ParticipantRow};

/// Общий SELECT списка чатов. Плейсхолдер `?1` — зритель: все агрегаты
/// (заголовок, последнее сообщение, непрочитанные, статус) считаются одним
/// запросом, чтобы список не уходил в N+1 по истории каждого диалога.
const CHAT_ROWS_SQL: &str = "
    SELECT d.id, d.type, d.title,
           (SELECT COALESCE(NULLIF(TRIM(u.display_name), ''), u.phone)
            FROM dialog_participants dp2
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

impl Database {
    // ===== ДИАЛОГИ =====

    /// Находит или создаёт личный диалог двух пользователей.
    ///
    /// Раньше JOIN без условия `dp1.user_id != dp2.user_id` при `user1 == user2`
    /// матчил любую строку участия и возвращал диалог с третьим человеком.
    fn find_private_dialog(
        conn: &rusqlite::Connection,
        user1: i64,
        user2: i64,
    ) -> Result<Option<i64>> {
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
    pub fn create_group_dialog(
        &self,
        title: &str,
        creator: i64,
        participants: &[i64],
    ) -> Result<i64> {
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
                        (SELECT COALESCE(NULLIF(TRIM(u.display_name), ''), u.phone)
                         FROM dialog_participants dp2
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
            is_group,
            title,
            participants,
        }))
    }

    pub fn get_dialog_participants(&self, dialog_id: i64) -> Result<Vec<i64>> {
        let conn = self.get_conn()?;
        let mut stmt =
            conn.prepare("SELECT user_id FROM dialog_participants WHERE dialog_id = ?1")?;
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
                    CASE WHEN us.status = 0 THEN 1 ELSE 0 END,
                    u.display_name
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
                display_name: row.get(4)?,
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
        tx.execute(
            "DELETE FROM dialogs WHERE id = ?1",
            rusqlite::params![dialog_id],
        )?;
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

    // ===== ЗАКРЕПЛЁННОЕ СООБЩЕНИЕ =====

    /// Ставит (`Some`) или снимает (`None`) закреплённое сообщение диалога.
    pub fn set_pinned_message(&self, dialog_id: i64, message_id: Option<i64>) -> Result<()> {
        let conn = self.get_conn()?;
        let updated = conn.execute(
            "UPDATE dialogs SET pinned_message_id = ?1 WHERE id = ?2",
            rusqlite::params![message_id, dialog_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("диалог {} не найден", dialog_id));
        }
        Ok(())
    }

    /// id закреплённого сообщения диалога.
    pub fn get_pinned_message_id(&self, dialog_id: i64) -> Result<Option<i64>> {
        let conn = self.get_conn()?;
        let pinned: Option<Option<i64>> = conn
            .query_row(
                "SELECT pinned_message_id FROM dialogs WHERE id = ?1",
                rusqlite::params![dialog_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(pinned.flatten())
    }

    /// Снимает закрепление, если закреплено именно это сообщение.
    pub fn clear_pinned_if_matches(&self, dialog_id: i64, message_id: i64) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute(
            "UPDATE dialogs SET pinned_message_id = NULL
             WHERE id = ?1 AND pinned_message_id = ?2",
            rusqlite::params![dialog_id, message_id],
        )?;
        Ok(())
    }
}

/// Человекочитаемый заголовок диалога.
///
/// Для личного диалога — телефон собеседника, для группы — сохранённое
/// название, а если его нет (старые записи) — "Группа {id}".
fn dialog_title(
    is_group: bool,
    raw_title: &str,
    other_phone: Option<&str>,
    dialog_id: i64,
) -> String {
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
