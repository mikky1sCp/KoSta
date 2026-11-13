// kosta-server/src/web/handlers/media.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::media;
use crate::rate_limiter::{check_limit, HTTP_LIMITER, MEDIA_LIMITER};
use crate::web::auth;
use crate::web::client_addr::client_ip;
use crate::web::models::{MessageItem, UploadMediaQuery};
use crate::web::state::WebState;

use super::{
    bad_request, dialog_participants, forbidden, not_found, now_secs, path_id, payload_too_large,
    percent_encode_filename, rate_limited, request_mime, server_error, unsupported_media_type,
    MAX_MEDIA_CAPTION_CHARS,
};

/// Загрузка файла и создание медиа-сообщения:
/// `POST /api/chats/{id}/media?name=<имя>&caption=<подпись>`.
///
/// Тело запроса — сырые байты файла; MIME берётся из `Content-Type`.
/// Файл сохраняется до вставки в БД, при ошибке вставки удаляется.
pub async fn upload_media(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
    path: web::Path<String>,
    query: web::Query<UploadMediaQuery>,
    body: web::Bytes,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    // Отдельный лимит по пользователю: загрузка пишет на диск и в историю.
    if let Err(e) = check_limit(&MEDIA_LIMITER, user_id) {
        return Ok(rate_limited(e));
    }

    let dialog_id = match path_id(&path, "id диалога") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let participants = match dialog_participants(&db, dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }

    let Some(raw_name) = query.name.as_deref() else {
        return Ok(bad_request("Параметр name обязателен"));
    };
    let name = match media::sanitize_filename(raw_name) {
        Ok(name) => name,
        Err(_) => return Ok(bad_request("Некорректное имя файла")),
    };
    let caption = query.caption.as_deref().unwrap_or("");
    if caption.chars().count() > MAX_MEDIA_CAPTION_CHARS {
        return Ok(bad_request(
            "Подпись слишком длинная: максимум 4096 символов",
        ));
    }

    let Some(mime) = request_mime(&req) else {
        return Ok(unsupported_media_type("Неподдерживаемый тип файла"));
    };
    if !media::is_supported_mime(&mime) {
        return Ok(unsupported_media_type("Неподдерживаемый тип файла"));
    }

    if body.is_empty() {
        return Ok(bad_request("Пустой файл"));
    }
    let max_bytes = media::max_bytes();
    if body.len() as u64 > max_bytes {
        return Ok(payload_too_large("Файл слишком большой"));
    }
    // Заголовок Content-Type приходит от клиента, поэтому сверяем его с
    // сигнатурой содержимого: иначе под `image/png` можно прислать что угодно.
    if !media::content_matches_mime(&body, &mime) {
        return Ok(unsupported_media_type(
            "Содержимое файла не соответствует типу",
        ));
    }

    let storage = match media::storage() {
        Ok(storage) => storage,
        Err(e) => return Ok(server_error("media storage is not initialized", e)),
    };
    let saved = match storage.save(dialog_id, &body, &name, &mime) {
        Ok(saved) => saved,
        Err(e) => return Ok(server_error("failed to store media file", e)),
    };

    let timestamp = now_secs();
    let size = body.len() as i64;
    let message_id = match db.save_media_message(
        dialog_id,
        user_id,
        caption,
        timestamp,
        &saved.key,
        &mime,
        size,
        &saved.name,
    ) {
        Ok(id) => id,
        Err(e) => {
            // Сообщения в истории нет — файл на диске стал бы сиротой.
            if let Err(cleanup) = storage.delete(&saved.key) {
                tracing::warn!(
                    "failed to clean up media file after db error: {:#}",
                    cleanup
                );
            }
            return Ok(server_error("failed to save media message", e));
        }
    };

    // Событие для WS-подписчиков диалога: is_outgoing всегда false, клиент
    // сравнивает sender_id со своим user_id (как в ws.rs).
    let event = json!({
        "type": "new_message",
        "payload": {
            "id": message_id,
            "dialog_id": dialog_id,
            "sender_id": user_id,
            "text": caption,
            "timestamp": timestamp,
            "is_outgoing": false,
            "read": false,
            "delivered": false,
            "media_name": &saved.name,
            "media_type": &mime,
            "media_size": size,
            "is_media": true,
        }
    })
    .to_string();
    state.broadcast_to_dialog(dialog_id, &event, None);

    Ok(HttpResponse::Ok().json(MessageItem {
        id: message_id,
        sender_id: user_id,
        text: caption.to_string(),
        timestamp,
        is_outgoing: true,
        read: false,
        delivered: false,
        media_name: Some(saved.name),
        media_type: Some(mime),
        media_size: Some(size),
        is_media: true,
        reply_to_id: None,
        reply_to_text: None,
        reply_to_sender_id: None,
        reply_to_is_media: false,
        edited_at: None,
        reactions: Vec::new(),
    }))
}

/// Скачивание файла медиа-сообщения: `GET /api/media/{message_id}`.
///
/// Доступно только участникам диалога, где лежит сообщение. Путь хранения
/// не фигурирует ни в ответе, ни в ошибках.
pub async fn download_media(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let message_id = match path_id(&path, "id сообщения") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let media_row = match db.get_media_message(message_id) {
        Ok(Some(row)) => row,
        Ok(None) => return Ok(not_found("media not found")),
        Err(e) => return Ok(server_error("failed to load media message", e)),
    };

    let participants = match dialog_participants(&db, media_row.dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }

    let storage = match media::storage() {
        Ok(storage) => storage,
        Err(e) => return Ok(server_error("media storage is not initialized", e)),
    };
    let bytes = match storage.read(&media_row.media_key) {
        Ok(bytes) => bytes,
        Err(e) => return Ok(server_error("failed to read media file", e)),
    };

    // Изображения показываем в браузере, остальное — только скачиванием.
    let disposition = if media_row.mime.starts_with("image/") {
        "inline".to_string()
    } else {
        let name = media_row.name.as_deref().unwrap_or("file");
        format!(
            "attachment; filename*=UTF-8''{}",
            percent_encode_filename(name)
        )
    };

    Ok(HttpResponse::Ok()
        .content_type(media_row.mime.as_str())
        .insert_header(("Content-Disposition", disposition.as_str()))
        .insert_header(("Cache-Control", "private"))
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .body(bytes))
}
