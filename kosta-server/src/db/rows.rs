// kosta-server/src/db/rows.rs
//! Строковые структуры, которые слой БД отдаёт наружу.

/// Строка истории сообщений.
///
/// Явные поля вместо позиционного кортежа: media-поля легко перепутать
/// местами (раньше `edited_at`/`deleted` уезжали в `media_size`/`is_media`).
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: i64,
    pub sender_id: i64,
    pub text: String,
    pub timestamp: i64,
    /// «Как видел отправитель»; web-слой пересчитывает для зрителя.
    /// Поле оставлено явным (и читается в тестах), чтобы не терять колонку
    /// при будущих запросах истории.
    #[allow(dead_code)]
    pub is_outgoing: bool,
    pub read: bool,
    pub delivered: bool,
    /// Ключ хранения в media-хранилище (`<dialog_id>/<hex>.<ext>`).
    /// Наружу через API не отдаётся.
    pub media_path: Option<String>,
    /// MIME-тип из белого списка (для Content-Type при скачивании).
    pub media_type: Option<String>,
    pub media_size: Option<i64>,
    pub media_name: Option<String>,
    /// Сообщение, на которое это отвечает (цитата).
    pub reply_to_id: Option<i64>,
    /// Текст цитируемого сообщения; `None`, если оно удалено или не найдено.
    pub reply_text: Option<String>,
    /// Автор цитируемого сообщения.
    pub reply_sender_id: Option<i64>,
    /// Было ли цитируемое сообщение медиа.
    pub reply_is_media: bool,
    /// Время последнего редактирования (unix seconds).
    pub edited_at: Option<i64>,
}

/// Реакция на сообщение, сгруппированная по эмодзи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionRow {
    pub emoji: String,
    pub count: i64,
    /// Ставил ли эту реакцию текущий зритель.
    pub me: bool,
}

impl HistoryRow {
    /// Медиа-сообщение: есть ключ хранения.
    pub fn is_media(&self) -> bool {
        self.media_path.is_some()
    }
}

/// Метаданные сообщения для рассылки правки/удаления.
#[derive(Debug, Clone)]
pub struct MessageMeta {
    pub dialog_id: i64,
    pub sender_id: i64,
    pub text: String,
    pub is_media: bool,
}

/// Метаданные медиа-сообщения для `GET /api/media/{message_id}`.
#[derive(Debug, Clone)]
pub struct MediaMessageRow {
    pub dialog_id: i64,
    pub media_key: String,
    pub mime: String,
    /// Размер полезен для диагностики/будущей проверки целостности;
    /// тело ответа отдаётся из файла, поэтому в хендлере не читается.
    #[allow(dead_code)]
    pub size: Option<i64>,
    pub name: Option<String>,
}

/// Строка списка чатов для web-API: заголовок уже разрешён
/// (телефон собеседника для личного диалога, название для группы).
#[derive(Debug, Clone)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub is_group: bool,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
    /// Непрочитанные входящие сообщения именно этого зрителя.
    pub unread_count: i64,
    /// Собеседник личного диалога; у группы всегда `None`.
    pub other_user_id: Option<i64>,
    /// Собеседник личного диалога сейчас онлайн (`user_status.status == 0`).
    /// Отсутствие строки в `user_status` означает offline.
    pub is_online: bool,
    /// Чат закреплён у зрителя (всегда наверху списка).
    pub pinned: bool,
    /// Уведомления отключены прямо сейчас (учитывается срок мьюта).
    pub muted: bool,
    /// До какого момента действует мьют (unix seconds; `0` — навсегда).
    /// `None` — мьют не включён.
    pub muted_until: Option<i64>,
    /// Чат убран в архив у зрителя.
    pub archived: bool,
    /// Зритель вручную пометил чат непрочитанным.
    pub marked_unread: bool,
}

/// Участник диалога с телефоном и статусом — для `GET /api/chats/{id}`.
#[derive(Debug, Clone)]
pub struct UserProfileRow {
    pub phone: String,
    pub display_name: Option<String>,
    pub avatar_key: Option<String>,
    pub is_online: bool,
    /// Публичный `@username` (если пользователь его задал).
    pub username: Option<String>,
}

/// Результат поиска людей: то, что можно безопасно показать любому
/// авторизованному пользователю. Телефон сюда не попадает — по username
/// можно найти человека, но не увидеть его номер.
#[derive(Debug, Clone)]
pub struct UserSearchRow {
    pub id: i64,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_key: Option<String>,
    pub is_online: bool,
}

/// Сообщение из глобального поиска: сама строка истории плюс диалог,
/// в котором она лежит (чтобы UI мог показать «куда перейти»).
#[derive(Debug, Clone)]
pub struct GlobalSearchHit {
    pub dialog_id: i64,
    pub title: String,
    pub row: HistoryRow,
}

#[derive(Debug, Clone)]
pub struct ParticipantRow {
    pub user_id: i64,
    pub phone: String,
    /// Отображаемое имя (если пользователь его задал).
    pub display_name: Option<String>,
    pub is_admin: bool,
    pub is_online: bool,
}

/// Метаданные диалога, нужные обработчикам TL и web-API.
#[derive(Debug, Clone)]
pub struct DialogMeta {
    pub is_group: bool,
    pub title: String,
    pub participants: Vec<i64>,
}

/// Сессия MTProto 2.0: сырой `auth_key`, salt и счётчики `msg_id`/`seq_no`.
#[derive(Debug, Clone)]
pub struct MtprotoSessionRow {
    /// `auth_key_id = SHA1(auth_key)[12..20]`.
    pub auth_key_id: i64,
    /// Пользователь (`0` — сессия ещё не прошла `signUp`).
    pub user_id: i64,
    /// Серверный salt для `new_session_created`.
    pub salt: i64,
    /// Сырой 2048-битный DH-секрет (256 байт big-endian).
    pub auth_key: [u8; 256],
    /// Последний принятый `msg_id`.
    pub last_msg_id: i64,
    /// Последний принятый `seq_no`.
    pub recv_seq_no: i32,
    /// Последний отправленный `seq_no`.
    pub send_seq_no: i32,
    /// Счётчик серверных `msg_id`.
    pub msg_id_counter: u32,
}

/// Собирает `HistoryRow` из строки запроса истории/поиска.
pub(super) fn history_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRow> {
    Ok(HistoryRow {
        id: row.get(0)?,
        sender_id: row.get(1)?,
        text: row.get(2)?,
        timestamp: row.get(3)?,
        is_outgoing: row.get::<_, i32>(4)? != 0,
        read: row.get::<_, i32>(5)? != 0,
        delivered: row.get::<_, i32>(6)? != 0,
        media_path: row.get(7)?,
        media_type: row.get(8)?,
        media_size: row.get(9)?,
        media_name: row.get(10)?,
        reply_to_id: row.get(11)?,
        edited_at: row.get(12)?,
        reply_text: row.get(13)?,
        reply_sender_id: row.get(14)?,
        reply_is_media: row.get::<_, Option<String>>(15)?.is_some(),
    })
}

/// Собирает `GlobalSearchHit` из строки глобального поиска.
///
/// Первые 16 колонок совпадают с `history_row_from_row`, затем идут
/// `dialog_id` и `title` — так один SQL-запрос отдаёт и сообщение, и диалог.
pub(super) fn global_search_row_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GlobalSearchHit> {
    Ok(GlobalSearchHit {
        dialog_id: row.get(16)?,
        title: row.get(17)?,
        row: history_row_from_row(row)?,
    })
}
