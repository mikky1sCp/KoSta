use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::rate_limiter::{check_limit, HTTP_LIMITER};
use crate::web::auth;
use crate::web::models::*;

fn client_ip(req: &HttpRequest) -> String {
    req.connection_info()
        .peer_addr()
        .unwrap_or("unknown")
        .to_string()
}

fn rate_limited(err: &str) -> HttpResponse {
    HttpResponse::TooManyRequests().json(json!({ "error": err }))
}

fn server_error(context: &str, err: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(json!({ "error": format!("{}: {}", context, err) }))
}

/// Вход либо регистрация по номеру телефона.
/// Существующий номер с неверным паролем — 401, а не создание второго аккаунта.
pub async fn login(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    json: web::Json<LoginRequest>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match db.get_user_id_by_phone(&json.phone) {
        Ok(Some(_)) => match db.authenticate(&json.phone, &json.password) {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                return Ok(HttpResponse::Unauthorized().json(LoginResponse {
                    success: false,
                    token: None,
                    auth_key_id: None,
                    user_id: None,
                    error: Some("Неверный номер или пароль".to_string()),
                }))
            }
            Err(e) => return Ok(server_error("database error", e)),
        },
        Ok(None) => match db.create_user(&json.phone, &json.password) {
            Ok(user_id) => user_id,
            Err(e) => return Ok(server_error("failed to create user", e)),
        },
        Err(e) => return Ok(server_error("database error", e)),
    };

    let token = match db.create_auth_token(user_id) {
        Ok(token) => token,
        Err(e) => return Ok(server_error("failed to issue token", e)),
    };

    // auth_key_id нужен только как справка о существующей KoSta-сессии.
    let auth_key_id = db
        .get_full_session_for_user(user_id)
        .ok()
        .flatten()
        .map(|session| session.auth_key_id);

    Ok(HttpResponse::Ok().json(LoginResponse {
        success: true,
        token: Some(token),
        auth_key_id,
        user_id: Some(user_id),
        error: None,
    }))
}

/// Отзывает текущий токен.
pub async fn logout(req: HttpRequest, db: web::Data<Arc<Database>>) -> Result<HttpResponse> {
    if let Some(token) = auth::token_from_request(&req) {
        if let Err(e) = db.revoke_token(&token) {
            return Ok(server_error("failed to revoke token", e));
        }
    }
    Ok(HttpResponse::Ok().json(json!({ "success": true })))
}

/// Список диалогов текущего пользователя (пользователь берётся из токена).
pub async fn get_chats(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let rows = match db.get_user_dialogs(user_id) {
        Ok(rows) => rows,
        Err(e) => return Ok(server_error("failed to load dialogs", e)),
    };

    let chats: Vec<ChatItem> = rows
        .into_iter()
        .map(|(id, title, typ, other_phone)| {
            let last = db
                .get_history(id, 0, 1)
                .ok()
                .and_then(|m| m.into_iter().next());
            let (last_message, last_timestamp) = match last {
                Some((_, _, text, ts, ..)) => (Some(text), Some(ts)),
                None => (None, None),
            };
            let title = if !title.is_empty() {
                title
            } else if !other_phone.is_empty() {
                other_phone
            } else {
                format!("Диалог {}", id)
            };
            ChatItem {
                id,
                title,
                is_group: typ == 1,
                last_message,
                last_timestamp,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(chats))
}

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
    // Ограничиваем диапазон: SQLite трактует отрицательный LIMIT как "без лимита".
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
            let messages: Vec<MessageItem> = rows
                .into_iter()
                .map(
                    |(id, sender_id, text, ts, out, read, delivered, media_path, media_type, media_size, is_media)| {
                        MessageItem {
                            id,
                            sender_id,
                            text,
                            timestamp: ts,
                            is_outgoing: out,
                            read,
                            delivered,
                            media_path,
                            media_type,
                            media_size,
                            is_media,
                        }
                    },
                )
                .collect();
            Ok(HttpResponse::Ok().json(messages))
        }
        Err(e) => Ok(server_error("failed to load history", e)),
    }
}
