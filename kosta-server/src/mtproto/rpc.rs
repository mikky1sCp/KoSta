//! Диспетчеризация прикладных RPC MTProto 2.0.
//!
//! Бизнес-логика переиспользует [`crate::db`], но работает с типами `kosta-tl`
//! вместо легаси-структур `kosta-core`. Каждый метод возвращает boxed TL-объект,
//! который оборачивается в `rpc_result` слоем соединения (или `rpc_error`).

use std::net::IpAddr;

use anyhow::Result;

use kosta_tl::{
    AddGroupParticipant, AuthResult, ChatMessage, CreateGroup, CreatePrivateDialog, DialogInfo,
    DialogList, GetDialogs, GetHistory, HistoryResult, OkResult, RemoveGroupParticipant,
    SendMessage, SignUp, TlObject, UpdateUserStatus, UserStatus,
};

use crate::db::{Database, HistoryRow};
use crate::mtproto::session::MtprotoSession;
use crate::rate_limiter::{check_limit, SIGNUP_LIMITER};

/// Максимальная длина текста сообщения (символы).
const MAX_MESSAGE_CHARS: usize = 4096;
/// Максимальная длина названия группы.
const MAX_GROUP_TITLE_CHARS: usize = 128;
/// Максимальное число участников группы.
const MAX_GROUP_PARTICIPANTS: usize = 200;

/// Прикладная RPC-ошибка (уходит клиенту в `rpc_error`).
#[derive(Debug, Clone)]
pub struct RpcError {
    /// Код ошибки.
    pub code: i32,
    /// Текст ошибки.
    pub message: String,
}

impl RpcError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        RpcError {
            code,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for RpcError {
    fn from(error: anyhow::Error) -> Self {
        // Детали внутренних ошибок не раскрываем клиенту.
        let _ = error;
        RpcError::new(500, "internal server error")
    }
}

fn ok_object() -> TlObject {
    TlObject::OkResult(OkResult)
}

fn is_valid_phone(phone: &str) -> bool {
    let digits = phone.strip_prefix('+').unwrap_or(phone);
    (5..=20).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
}

fn is_valid_password(password: &str) -> bool {
    (8..=128).contains(&password.chars().count())
}

fn require_user(session: &MtprotoSession) -> Result<i64, RpcError> {
    if session.user_id == 0 {
        return Err(RpcError::new(401, "session is not signed up"));
    }
    Ok(session.user_id)
}

fn ensure_participant(db: &Database, dialog_id: i64, user_id: i64) -> Result<(), RpcError> {
    let participants = db.get_dialog_participants(dialog_id)?;
    if participants.is_empty() {
        return Err(RpcError::new(404, "dialog not found"));
    }
    if !participants.contains(&user_id) {
        return Err(RpcError::new(403, "not a participant of this dialog"));
    }
    Ok(())
}

fn chat_message(row: HistoryRow, dialog_id: i64, viewer: i64) -> ChatMessage {
    ChatMessage::ChatMessage {
        id: row.id,
        chat_id: dialog_id,
        sender_id: row.sender_id,
        text: row.text,
        timestamp: row.timestamp,
        is_outgoing: row.sender_id == viewer,
        read: row.read,
        delivered: row.delivered,
    }
}

fn dialog_info(db: &Database, dialog_id: i64, viewer: i64) -> Result<DialogInfo, RpcError> {
    let meta = db
        .get_dialog_meta(dialog_id, viewer)?
        .ok_or_else(|| RpcError::new(404, "dialog not found"))?;

    let last_message = match db.get_history(dialog_id, 0, 1)?.into_iter().next() {
        Some(row) => chat_message(row, dialog_id, viewer),
        None => ChatMessage::ChatMessageEmpty,
    };
    let unread_count = db
        .get_chat_row(viewer, dialog_id)?
        .map(|row| row.unread_count as i32)
        .unwrap_or(0);

    Ok(DialogInfo {
        id: dialog_id,
        title: meta.title,
        is_group: meta.is_group,
        participants: meta.participants,
        last_message,
        unread_count,
    })
}

/// Выполняет прикладной RPC.
pub fn dispatch(
    db: &Database,
    session: &mut MtprotoSession,
    object: TlObject,
    peer: IpAddr,
) -> Result<TlObject, RpcError> {
    match object {
        TlObject::SignUp(request) => handle_sign_up(db, session, request, peer),
        TlObject::SendMessage(request) => handle_send_message(db, session, request),
        TlObject::GetHistory(request) => handle_get_history(db, session, request),
        TlObject::GetDialogs(request) => handle_get_dialogs(db, session, request),
        TlObject::CreatePrivateDialog(request) => {
            handle_create_private_dialog(db, session, request)
        }
        TlObject::CreateGroup(request) => handle_create_group(db, session, request),
        TlObject::AddGroupParticipant(request) => handle_add_participant(db, session, request),
        TlObject::RemoveGroupParticipant(request) => {
            handle_remove_participant(db, session, request)
        }
        TlObject::AckMessage(request) => handle_ack_message(db, session, request),
        TlObject::UpdateUserStatus(request) => handle_update_status(db, session, request),
        other => Err(RpcError::new(
            400,
            format!("unsupported RPC object: {}", other.constructor_id()),
        )),
    }
}

fn handle_sign_up(
    db: &Database,
    session: &mut MtprotoSession,
    request: SignUp,
    peer: IpAddr,
) -> Result<TlObject, RpcError> {
    check_limit(&SIGNUP_LIMITER, peer.to_string())
        .map_err(|_| RpcError::new(429, "sign up rate limit exceeded"))?;
    if !is_valid_phone(&request.phone) {
        return Err(RpcError::new(400, "invalid phone number"));
    }
    if !is_valid_password(&request.password) {
        return Err(RpcError::new(400, "password must be 8..=128 characters"));
    }
    if db.get_user_id_by_phone(&request.phone)?.is_some() {
        return Err(RpcError::new(409, "phone number is already registered"));
    }

    let user_id = db.create_user(&request.phone, &request.password)?;
    db.update_mtproto_session_user_id(session.auth_key_id, user_id)?;
    session.user_id = user_id;
    Ok(TlObject::AuthResult(AuthResult {
        user_id,
        auth_key_id: session.auth_key_id,
    }))
}

fn handle_send_message(
    db: &Database,
    session: &MtprotoSession,
    request: SendMessage,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    ensure_participant(db, request.chat_id, user_id)?;

    let text = request.text.trim();
    if text.is_empty() {
        return Err(RpcError::new(400, "message text is empty"));
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(RpcError::new(
            400,
            format!("message is too long (max {MAX_MESSAGE_CHARS} chars)"),
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let message_id = db.save_message(request.chat_id, user_id, text, now, true, None, None)?;

    Ok(TlObject::ChatMessage(ChatMessage::ChatMessage {
        id: message_id,
        chat_id: request.chat_id,
        sender_id: user_id,
        text: text.to_string(),
        timestamp: now,
        is_outgoing: true,
        read: false,
        delivered: false,
    }))
}

fn handle_get_history(
    db: &Database,
    session: &MtprotoSession,
    request: GetHistory,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    ensure_participant(db, request.chat_id, user_id)?;

    let limit = request.limit.clamp(1, 200);
    let offset = request.offset.max(0);
    let rows = db.get_history(request.chat_id, offset, limit)?;
    let total_count = rows.len() as i32;
    let messages = rows
        .into_iter()
        .map(|row| chat_message(row, request.chat_id, user_id))
        .collect();
    Ok(TlObject::HistoryResult(HistoryResult {
        messages,
        total_count,
    }))
}

fn handle_get_dialogs(
    db: &Database,
    session: &MtprotoSession,
    _request: GetDialogs,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    let chats = db.get_user_chats(user_id)?;
    let mut dialogs = Vec::with_capacity(chats.len());
    for chat in chats {
        dialogs.push(dialog_info(db, chat.id, user_id)?);
    }
    Ok(TlObject::DialogList(DialogList { dialogs }))
}

fn handle_create_private_dialog(
    db: &Database,
    session: &MtprotoSession,
    request: CreatePrivateDialog,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    if request.user_id == user_id {
        return Err(RpcError::new(
            400,
            "cannot create a private dialog with yourself",
        ));
    }
    if !db.user_exists(request.user_id)? {
        return Err(RpcError::new(404, "user not found"));
    }
    let dialog_id = db.create_private_dialog(user_id, request.user_id)?;
    Ok(TlObject::DialogInfo(dialog_info(db, dialog_id, user_id)?))
}

fn handle_create_group(
    db: &Database,
    session: &MtprotoSession,
    request: CreateGroup,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;

    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > MAX_GROUP_TITLE_CHARS {
        return Err(RpcError::new(
            400,
            format!("group title must be 1..={MAX_GROUP_TITLE_CHARS} characters"),
        ));
    }

    let mut participants: Vec<i64> = Vec::new();
    for participant in request.participants {
        if participant == user_id || participants.contains(&participant) {
            continue;
        }
        participants.push(participant);
    }
    if participants.len() + 1 > MAX_GROUP_PARTICIPANTS {
        return Err(RpcError::new(
            400,
            format!("too many participants (max {MAX_GROUP_PARTICIPANTS})"),
        ));
    }
    for participant in participants.iter() {
        if !db.user_exists(*participant)? {
            return Err(RpcError::new(
                404,
                format!("participant {participant} not found"),
            ));
        }
    }

    let dialog_id = db.create_group_dialog(title, user_id, &participants)?;
    Ok(TlObject::DialogInfo(dialog_info(db, dialog_id, user_id)?))
}

fn handle_add_participant(
    db: &Database,
    session: &MtprotoSession,
    request: AddGroupParticipant,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    let dialog_id = request.dialog_id;

    let dialog_type = db
        .get_dialog_type(dialog_id)?
        .ok_or_else(|| RpcError::new(404, "dialog not found"))?;
    if dialog_type != 1 {
        return Err(RpcError::new(400, "not a group dialog"));
    }
    if !db.is_dialog_admin(dialog_id, user_id)? {
        return Err(RpcError::new(403, "only group admins can add participants"));
    }
    if !db.user_exists(request.user_id)? {
        return Err(RpcError::new(404, "target user not found"));
    }
    let participants = db.get_dialog_participants(dialog_id)?;
    if participants.len() >= MAX_GROUP_PARTICIPANTS {
        return Err(RpcError::new(
            400,
            format!("group is full (max {MAX_GROUP_PARTICIPANTS})"),
        ));
    }
    if !participants.contains(&request.user_id) {
        db.add_participant(dialog_id, request.user_id)?;
    }
    Ok(TlObject::DialogInfo(dialog_info(db, dialog_id, user_id)?))
}

fn handle_remove_participant(
    db: &Database,
    session: &MtprotoSession,
    request: RemoveGroupParticipant,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    let dialog_id = request.dialog_id;

    let meta = db
        .get_dialog_meta(dialog_id, user_id)?
        .ok_or_else(|| RpcError::new(404, "dialog not found"))?;
    if !meta.is_group {
        return Err(RpcError::new(400, "not a group dialog"));
    }
    if !meta.participants.contains(&user_id) {
        return Err(RpcError::new(403, "you are not a member of this group"));
    }

    if request.user_id == user_id {
        db.remove_participant(dialog_id, user_id)?;
    } else {
        if !db.is_dialog_admin(dialog_id, user_id)? {
            return Err(RpcError::new(
                403,
                "only group admins can remove participants",
            ));
        }
        if !meta.participants.contains(&request.user_id) {
            return Err(RpcError::new(404, "user is not a participant"));
        }
        db.remove_participant(dialog_id, request.user_id)?;
    }

    db.ensure_group_has_admin(dialog_id)?;

    if db.get_dialog_participants(dialog_id)?.is_empty() {
        let info = DialogInfo {
            id: dialog_id,
            title: meta.title,
            is_group: true,
            participants: Vec::new(),
            last_message: ChatMessage::ChatMessageEmpty,
            unread_count: 0,
        };
        db.delete_empty_dialog(dialog_id)?;
        if let Err(error) = crate::media::delete_dialog(dialog_id) {
            tracing::warn!("failed to delete media of empty dialog {dialog_id}: {error:#}");
        }
        return Ok(TlObject::DialogInfo(info));
    }

    Ok(TlObject::DialogInfo(dialog_info(db, dialog_id, user_id)?))
}

fn handle_ack_message(
    db: &Database,
    session: &MtprotoSession,
    request: kosta_tl::AckMessage,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    let dialog_id = db
        .get_message_dialog_id(request.message_id)?
        .ok_or_else(|| RpcError::new(404, "message not found"))?;
    ensure_participant(db, dialog_id, user_id)?;
    db.mark_message_delivered(request.message_id)?;
    Ok(ok_object())
}

fn handle_update_status(
    db: &Database,
    session: &MtprotoSession,
    request: UpdateUserStatus,
) -> Result<TlObject, RpcError> {
    let user_id = require_user(session)?;
    if user_id != request.user_id {
        return Err(RpcError::new(403, "cannot update status for another user"));
    }
    let status = match request.status {
        UserStatus::UserStatusOnline => 0,
        UserStatus::UserStatusOffline => 1,
        UserStatus::UserStatusTyping => 2,
        UserStatus::UserStatusRecently => 3,
        UserStatus::UserStatusLastWeek => 4,
        UserStatus::UserStatusLastMonth => 5,
    };
    db.set_user_status(user_id, status)?;
    Ok(ok_object())
}
