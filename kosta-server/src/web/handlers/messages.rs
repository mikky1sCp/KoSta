// kosta-server/src/web/handlers/messages.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::rate_limiter::{check_limit, HTTP_LIMITER};
use crate::web::auth;
use crate::web::client_addr::client_ip;
use crate::web::models::{GetHistoryQuery, GlobalSearchHitItem, MessageItem, ReactionItem};
use crate::web::state::WebState;

use super::{
    bad_request, dialog_participants, forbidden, not_found, path_id, rate_limited, server_error,
    MAX_MEDIA_CAPTION_CHARS, MAX_REACTION_CHARS, MAX_SEARCH_QUERY_CHARS,
};

/// История диалога. Доступна только участникам диалога.
pub async fn get_history(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    query: web::Query<GetHistoryQuery>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let chat_id = query.chat_id;
    // Защита в глубину: db тоже клампит, но не полагаемся на единственное место.
    // SQLite трактует отрицательный LIMIT как «без лимита».
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);

    let participants = match db.get_dialog_participants(chat_id) {
        Ok(p) => p,
        Err(e) => return Ok(server_error("failed to load dialog", e)),
    };
    if participants.is_empty() {
        return Ok(HttpResponse::NotFound().json(json!({ "error": "dialog not found" })));
    }
    if !participants.contains(&user_id) {
        return Ok(HttpResponse::Forbidden().json(json!({ "error": "not a participant" })));
    }

    match db.get_history(chat_id, offset, limit) {
        Ok(rows) => {
            let mut messages: Vec<MessageItem> = rows
                .into_iter()
                .map(|row| message_item_from_row(row, user_id))
                .collect();
            attach_reactions(&db, chat_id, user_id, &mut messages);
            Ok(HttpResponse::Ok().json(messages))
        }
        Err(e) => Ok(server_error("failed to load history", e)),
    }
}

/// Строка БД → элемент API глазами зрителя.
pub(super) fn message_item_from_row(row: crate::db::HistoryRow, viewer_id: i64) -> MessageItem {
    // Считаем до перемещения полей: `is_media` зависит от `media_path`.
    let is_media = row.is_media();
    MessageItem {
        id: row.id,
        sender_id: row.sender_id,
        text: row.text,
        timestamp: row.timestamp,
        // Одна колонка is_outgoing хранит «как видел отправитель»;
        // для зрителя исходящее = отправлено им самим.
        is_outgoing: row.sender_id == viewer_id,
        read: row.read,
        delivered: row.delivered,
        media_name: row.media_name,
        media_type: row.media_type,
        media_size: row.media_size,
        is_media,
        reply_to_id: row.reply_to_id,
        reply_to_text: row.reply_text,
        reply_to_sender_id: row.reply_sender_id,
        reply_to_is_media: row.reply_is_media,
        edited_at: row.edited_at,
        reactions: Vec::new(),
    }
}

/// Подмешивает агрегированные реакции к сообщениям одного диалога.
fn attach_reactions(db: &Database, dialog_id: i64, viewer_id: i64, messages: &mut [MessageItem]) {
    if messages.is_empty() {
        return;
    }
    match db.get_dialog_reactions(dialog_id, viewer_id) {
        Ok(by_message) => {
            for item in messages.iter_mut() {
                if let Some(list) = by_message.get(&item.id) {
                    item.reactions = list
                        .iter()
                        .map(|reaction| ReactionItem {
                            emoji: reaction.emoji.clone(),
                            count: reaction.count,
                            me: reaction.me,
                        })
                        .collect();
                }
            }
        }
        Err(e) => tracing::error!("failed to load reactions for dialog {}: {}", dialog_id, e),
    }
}

/// Параметры `GET /api/search`.
///
/// `chat_id` необязателен: без него поиск идёт по всем диалогам пользователя
/// (глобальный поиск), с ним — только внутри указанного диалога.
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    pub chat_id: Option<i64>,
    pub q: String,
    #[serde(default)]
    pub limit: Option<i32>,
}

/// Поиск по сообщениям.
///
/// `GET /api/search?chat_id=12&q=текст` — только внутри диалога (как раньше);
/// `GET /api/search?q=текст` — по всем диалогам пользователя.
pub async fn search_history(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    query: web::Query<SearchQuery>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }
    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let needle = query.q.trim();
    if needle.is_empty() {
        return Ok(bad_request("query is empty"));
    }
    if needle.chars().count() > MAX_SEARCH_QUERY_CHARS {
        return Ok(bad_request("query is too long (max 128 characters)"));
    }

    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    match query.chat_id {
        Some(chat_id) => {
            let participants = match dialog_participants(&db, chat_id) {
                Ok(participants) => participants,
                Err(resp) => return Ok(resp),
            };
            if !participants.contains(&user_id) {
                return Ok(forbidden("not a participant"));
            }

            match db.search_messages(chat_id, needle, limit) {
                Ok(rows) => {
                    let mut messages: Vec<MessageItem> = rows
                        .into_iter()
                        .map(|row| message_item_from_row(row, user_id))
                        .collect();
                    attach_reactions(&db, chat_id, user_id, &mut messages);
                    Ok(HttpResponse::Ok().json(messages))
                }
                Err(e) => Ok(server_error("failed to search messages", e)),
            }
        }
        None => {
            let hits = match db.search_messages_global(user_id, needle, limit) {
                Ok(hits) => hits,
                Err(e) => return Ok(server_error("failed to search messages", e)),
            };
            // Реакции грузим один раз на диалог, а не на каждое сообщение.
            let mut reactions_by_dialog: std::collections::HashMap<
                i64,
                std::collections::HashMap<i64, Vec<crate::db::ReactionRow>>,
            > = std::collections::HashMap::new();
            for hit in &hits {
                if reactions_by_dialog.contains_key(&hit.dialog_id) {
                    continue;
                }
                match db.get_dialog_reactions(hit.dialog_id, user_id) {
                    Ok(by_message) => {
                        reactions_by_dialog.insert(hit.dialog_id, by_message);
                    }
                    Err(e) => {
                        tracing::error!(
                            "failed to load reactions for dialog {}: {}",
                            hit.dialog_id,
                            e
                        );
                    }
                }
            }

            let items: Vec<GlobalSearchHitItem> = hits
                .into_iter()
                .map(|hit| {
                    let mut message = message_item_from_row(hit.row, user_id);
                    if let Some(by_message) = reactions_by_dialog.get(&hit.dialog_id) {
                        if let Some(list) = by_message.get(&message.id) {
                            message.reactions = list
                                .iter()
                                .map(|reaction| ReactionItem {
                                    emoji: reaction.emoji.clone(),
                                    count: reaction.count,
                                    me: reaction.me,
                                })
                                .collect();
                        }
                    }
                    GlobalSearchHitItem {
                        chat_id: hit.dialog_id,
                        chat_title: hit.title,
                        message,
                    }
                })
                .collect();
            Ok(HttpResponse::Ok().json(items))
        }
    }
}

/// Тело `PATCH /api/messages/{id}`.
#[derive(Debug, serde::Deserialize)]
pub struct EditMessageBody {
    pub text: String,
}

/// Редактирование своего сообщения: `PATCH /api/messages/{id}`.
///
/// Правит только автор; подписчикам диалога уходит `message_edited`,
/// чтобы открытые клиенты обновили текст без перезагрузки истории.
pub async fn edit_message(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
    path: web::Path<String>,
    body: web::Json<EditMessageBody>,
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

    let text = body.text.trim();
    if text.is_empty() {
        return Ok(bad_request("message text is empty"));
    }
    if text.chars().count() > MAX_MEDIA_CAPTION_CHARS {
        return Ok(bad_request(
            "message text is too long (max 4096 characters)",
        ));
    }

    let meta = match db.get_message_meta(message_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => return Ok(not_found("message not found")),
        Err(e) => return Ok(server_error("failed to load message", e)),
    };
    let participants = match dialog_participants(&db, meta.dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }
    if meta.sender_id != user_id {
        return Ok(forbidden("only the sender can edit the message"));
    }

    let edited_at = match db.edit_message(message_id, user_id, text) {
        Ok(edited_at) => edited_at,
        Err(e) => return Ok(server_error("failed to edit message", e)),
    };

    let event = json!({
        "type": "message_edited",
        "payload": {
            "dialog_id": meta.dialog_id,
            "message_id": message_id,
            "text": text,
            "edited_at": edited_at,
        }
    })
    .to_string();
    state.broadcast_to_dialog(meta.dialog_id, &event, None);

    // Если отредактировано закреплённое сообщение — обновим и плашку.
    if matches!(db.get_pinned_message_id(meta.dialog_id), Ok(Some(id)) if id == message_id) {
        if let Ok(Some(row)) = db.get_message_row(message_id) {
            let pinned = json!({
                "type": "dialog_pinned",
                "payload": {
                    "dialog_id": meta.dialog_id,
                    "message": message_item_from_row(row, user_id),
                }
            })
            .to_string();
            state.broadcast_to_dialog(meta.dialog_id, &pinned, None);
        }
    }

    Ok(HttpResponse::Ok().json(json!({ "ok": true, "edited_at": edited_at })))
}

/// Удаление своего сообщения: `DELETE /api/messages/{id}`.
///
/// Мягкое удаление: строка остаётся в БД (`deleted = 1`), история её больше
/// не отдаёт. Подписчикам уходит `message_deleted`, и клиенты убирают пузырь.
pub async fn delete_message(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
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

    let meta = match db.get_message_meta(message_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => return Ok(not_found("message not found")),
        Err(e) => return Ok(server_error("failed to load message", e)),
    };
    let participants = match dialog_participants(&db, meta.dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }
    if meta.sender_id != user_id {
        return Ok(forbidden("only the sender can delete the message"));
    }

    if let Err(e) = db.delete_message(message_id, user_id) {
        return Ok(server_error("failed to delete message", e));
    }

    // Удалённое сообщение не должно остаться в закреплённых.
    let was_pinned =
        matches!(db.get_pinned_message_id(meta.dialog_id), Ok(Some(id)) if id == message_id);
    if was_pinned {
        if let Err(e) = db.clear_pinned_if_matches(meta.dialog_id, message_id) {
            tracing::error!("failed to clear pinned message: {}", e);
        }
        let unpin_event = json!({
            "type": "dialog_pinned",
            "payload": { "dialog_id": meta.dialog_id, "message": serde_json::Value::Null }
        })
        .to_string();
        state.broadcast_to_dialog(meta.dialog_id, &unpin_event, None);
    }

    let event = json!({
        "type": "message_deleted",
        "payload": {
            "dialog_id": meta.dialog_id,
            "message_id": message_id,
        }
    })
    .to_string();
    state.broadcast_to_dialog(meta.dialog_id, &event, None);

    Ok(HttpResponse::Ok().json(json!({ "ok": true })))
}

/// Тело `POST /api/messages/{id}/reactions`.
#[derive(Debug, serde::Deserialize)]
pub struct ReactionBody {
    pub emoji: String,
}

/// Переключает реакцию на сообщение: `POST /api/messages/{id}/reactions`.
///
/// Повторный вызов с тем же эмодзи снимает реакцию. В рассылку уходят только
/// эмодзи и счётчики: флаг «моя реакция» у каждого зрителя свой.
pub async fn toggle_reaction(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
    path: web::Path<String>,
    body: web::Json<ReactionBody>,
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

    let emoji = body.emoji.trim();
    if emoji.is_empty() {
        return Ok(bad_request("emoji is empty"));
    }
    if emoji.chars().count() > MAX_REACTION_CHARS {
        return Ok(bad_request("emoji is too long"));
    }
    if emoji.chars().any(|c| c.is_control()) {
        return Ok(bad_request("emoji contains control characters"));
    }

    let meta = match db.get_message_meta(message_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => return Ok(not_found("message not found")),
        Err(e) => return Ok(server_error("failed to load message", e)),
    };
    let participants = match dialog_participants(&db, meta.dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }

    // Один эндпоинт и ставит, и снимает реакцию.
    match db.add_reaction(message_id, user_id, emoji) {
        Ok(true) => {}
        Ok(false) => {
            if let Err(e) = db.remove_reaction(message_id, user_id, emoji) {
                return Ok(server_error("failed to remove reaction", e));
            }
        }
        Err(e) => return Ok(server_error("failed to add reaction", e)),
    }

    let reactions = match db.get_message_reactions(message_id, user_id) {
        Ok(reactions) => reactions,
        Err(e) => return Ok(server_error("failed to load reactions", e)),
    };

    // В рассылке нет `me`: у каждого подписчика этот флаг свой.
    let broadcast_reactions: Vec<_> = reactions
        .iter()
        .map(|reaction| json!({ "emoji": reaction.emoji, "count": reaction.count }))
        .collect();
    let event = json!({
        "type": "message_reactions",
        "payload": {
            "dialog_id": meta.dialog_id,
            "message_id": message_id,
            "reactions": broadcast_reactions,
        }
    })
    .to_string();
    state.broadcast_to_dialog(meta.dialog_id, &event, None);

    let response_reactions: Vec<ReactionItem> = reactions
        .into_iter()
        .map(|reaction| ReactionItem {
            emoji: reaction.emoji,
            count: reaction.count,
            me: reaction.me,
        })
        .collect();

    Ok(HttpResponse::Ok().json(json!({ "ok": true, "reactions": response_reactions })))
}
