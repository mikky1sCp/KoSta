// kosta-server/src/web/handlers/mod.rs
mod auth;
mod chats;
mod media;
mod messages;
mod profile;
#[cfg(test)]
mod tests;

pub use auth::{login, logout};
pub use chats::{
    add_participant, create_group_chat, create_private_chat, get_chat, get_chats, mark_chat_read,
    pin_message, remove_participant, rename_chat,
};
pub use media::{download_media, upload_media};
pub use messages::{delete_message, edit_message, get_history, search_history, toggle_reaction};
pub use profile::{get_avatar, get_me, lookup_user, update_me, upload_avatar};

// Тела запросов остаются частью публичного API модуля (на них ссылаются
// сигнатуры хендлеров), но по пути реэкспорта в бинарном крейте не читаются.
#[allow(unused_imports)]
pub use chats::PinMessageBody;
#[allow(unused_imports)]
pub use messages::{EditMessageBody, ReactionBody, SearchQuery};
#[allow(unused_imports)]
pub use profile::UpdateProfileBody;

use actix_web::{http::header, HttpRequest, HttpResponse};
use anyhow::Result as AnyhowResult;
use serde_json::json;

use crate::db::{ChatRow, Database, ParticipantRow};
use crate::web::models::{ChatDetails, ChatItem, LoginResponse, ParticipantItem};

use self::messages::message_item_from_row;

/// Максимум участников группы, включая создателя (как в TL-обработчиках).
const MAX_GROUP_PARTICIPANTS: usize = 200;
/// Создатель добавляется сам, поэтому в теле создания группы — до 199 номеров.
const MAX_GROUP_INVITEES: usize = MAX_GROUP_PARTICIPANTS - 1;
/// Максимальная длина названия группы в символах (не байтах).
const MAX_GROUP_TITLE_CHARS: usize = 128;
/// Максимальная длина подписи к медиа — та же, что у текста сообщения.
const MAX_MEDIA_CAPTION_CHARS: usize = 4096;
/// Максимальная длина отображаемого имени в профиле.
const MAX_DISPLAY_NAME_CHARS: usize = 64;
/// Максимальная длина поискового запроса.
const MAX_SEARCH_QUERY_CHARS: usize = 128;
/// Максимальная длина эмодзи-реакции в символах (эмодзи бывают составными).
const MAX_REACTION_CHARS: usize = 8;

fn rate_limited(err: &str) -> HttpResponse {
    HttpResponse::TooManyRequests().json(json!({ "error": err }))
}

/// Внутренние ошибки — только в лог: клиенту SQL и структура БД не нужны.
fn server_error(context: &str, err: impl std::fmt::Display) -> HttpResponse {
    tracing::error!("{}: {}", context, err);
    HttpResponse::InternalServerError().json(json!({ "error": "internal server error" }))
}

fn bad_request(message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "error": message }))
}

fn not_found(message: &str) -> HttpResponse {
    HttpResponse::NotFound().json(json!({ "error": message }))
}

fn forbidden(message: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(json!({ "error": message }))
}

fn payload_too_large(message: &str) -> HttpResponse {
    HttpResponse::PayloadTooLarge().json(json!({ "error": message }))
}

fn unsupported_media_type(message: &str) -> HttpResponse {
    HttpResponse::UnsupportedMediaType().json(json!({ "error": message }))
}

/// Текущее время в unix-секундах (для timestamp сообщения).
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// MIME из заголовка `Content-Type`, без параметров (`; charset=...`) и в
/// нижнем регистре. Отсутствующий/битый заголовок — `None` (на уровне API 415).
fn request_mime(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|mime| !mime.is_empty())
}

/// Percent-encoding для `filename*` в Content-Disposition (RFC 5987).
///
/// Оставляем только unreserved-символы: юникод и пробелы уходят в `%XX`,
/// поэтому заголовок остаётся ASCII и не ломает ответ.
fn percent_encode_filename(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

/// Числовой id из пути. Разбираем вручную, чтобы ошибка была в общем
/// JSON-формате: экстрактор `web::Path<i64>` на мусоре отдаёт текстовый 400.
#[allow(clippy::result_large_err)] // готовый HttpResponse — самый прямой путь
fn path_id(raw: &str, what: &str) -> std::result::Result<i64, HttpResponse> {
    raw.parse::<i64>()
        .map_err(|_| bad_request(&format!("Некорректный {}", what)))
}

/// Участники существующего диалога или готовый ответ: 404 — диалога нет,
/// 500 — сбой БД.
#[allow(clippy::result_large_err)] // как и в auth.rs: возвращаем готовый ответ
fn dialog_participants(
    db: &Database,
    dialog_id: i64,
) -> std::result::Result<Vec<i64>, HttpResponse> {
    match db.get_dialog_participants(dialog_id) {
        Ok(participants) if participants.is_empty() => Err(not_found("dialog not found")),
        Ok(participants) => Ok(participants),
        Err(e) => Err(server_error("failed to load dialog", e)),
    }
}

/// Карточка чата из строки БД. У группы собеседника нет, поэтому
/// `is_online` в JSON не попадает (None), а не приходит как false.
fn chat_item(row: ChatRow) -> ChatItem {
    ChatItem {
        id: row.id,
        title: row.title,
        is_group: row.is_group,
        last_message: row.last_message,
        last_timestamp: row.last_timestamp,
        unread_count: row.unread_count,
        other_user_id: row.other_user_id,
        is_online: (!row.is_group).then_some(row.is_online),
    }
}

fn participant_item(row: ParticipantRow) -> ParticipantItem {
    ParticipantItem {
        user_id: row.user_id,
        phone: row.phone,
        display_name: row.display_name,
        is_admin: row.is_admin,
        is_online: row.is_online,
    }
}

/// Собирает `ChatDetails` глазами зрителя. `None` — карточки нет (зритель
/// не участник или диалог успели удалить).
fn build_chat_details(
    db: &Database,
    viewer_id: i64,
    dialog_id: i64,
) -> AnyhowResult<Option<ChatDetails>> {
    let Some(row) = db.get_chat_row(viewer_id, dialog_id)? else {
        return Ok(None);
    };
    let participants = db.get_dialog_participants_detailed(dialog_id)?;
    let is_admin = db.is_dialog_admin(dialog_id, viewer_id)?;
    // Закреплённое сообщение: удалённое или отсутствующее просто не показываем.
    let pinned_message = match db.get_pinned_message_id(dialog_id)? {
        Some(message_id) => db
            .get_message_row(message_id)?
            .map(|row| message_item_from_row(row, viewer_id)),
        None => None,
    };
    Ok(Some(ChatDetails {
        chat: chat_item(row),
        participants: participants.into_iter().map(participant_item).collect(),
        is_admin,
        pinned_message,
    }))
}

/// Готовый 200-ответ с деталями диалога (или 404/500, если их не собрать).
fn chat_details_response(db: &Database, viewer_id: i64, dialog_id: i64) -> HttpResponse {
    match build_chat_details(db, viewer_id, dialog_id) {
        Ok(Some(details)) => HttpResponse::Ok().json(details),
        Ok(None) => not_found("dialog not found"),
        Err(e) => server_error("failed to load chat details", e),
    }
}

/// Один и тот же ответ и для «нет такого номера», и для неверного пароля:
/// иначе по тексту ошибки видно, зарегистрирован ли телефон.
fn invalid_credentials() -> HttpResponse {
    HttpResponse::Unauthorized().json(LoginResponse {
        success: false,
        token: None,
        auth_key_id: None,
        user_id: None,
        error: Some("Неверный номер или пароль".to_string()),
    })
}

/// Телефон: необязательный «+» и 5–20 цифр. Номер — ключ аккаунта,
/// поэтому формат строгий: иначе один и тот же человек получит два аккаунта.
fn valid_phone(phone: &str) -> bool {
    let digits = phone.strip_prefix('+').unwrap_or(phone);
    (5..=20).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
}

/// Пароль: 8–128 символов (считаем символы, а не байты — UI тоже так думает).
fn valid_password(password: &str) -> bool {
    (8..=128).contains(&password.chars().count())
}
