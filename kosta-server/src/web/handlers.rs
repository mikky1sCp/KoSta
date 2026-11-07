// kosta-server/src/web/handlers.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::rate_limiter::{check_limit, HTTP_LIMITER, LOGIN_LIMITER};
use crate::web::auth;
use crate::web::models::*;
use crate::web::state::WebState;

fn client_ip(req: &HttpRequest) -> String {
    req.connection_info()
        .peer_addr()
        .unwrap_or("unknown")
        .to_string()
}

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

/// Вход либо регистрация по номеру телефона.
/// Существующий номер с неверным паролем — 401, а не создание второго аккаунта.
/// Регистрация только с явным `register: true`: иначе API работал бы как
/// перечисление аккаунтов и авторегистрация без согласия пользователя.
pub async fn login(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    json: web::Json<LoginRequest>,
) -> Result<HttpResponse> {
    // Логин дорогой (bcrypt), поэтому лимит отдельный и жёстче общего HTTP.
    if let Err(e) = check_limit(&LOGIN_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    if !valid_phone(&json.phone) {
        return Ok(bad_request("Некорректный номер телефона"));
    }
    if !valid_password(&json.password) {
        return Ok(bad_request("Пароль должен быть от 8 до 128 символов"));
    }

    let existing = match db.get_user_id_by_phone(&json.phone) {
        Ok(user_id) => user_id,
        Err(e) => return Ok(server_error("database error", e)),
    };

    let user_id = match existing {
        Some(_) => match db.authenticate(&json.phone, &json.password) {
            Ok(Some(user_id)) => user_id,
            // Битый/пустой password_hash (например, системный пользователь)
            // тоже считается неверными данными входа.
            Ok(None) => return Ok(invalid_credentials()),
            Err(e) => return Ok(server_error("database error", e)),
        },
        None if json.register => match db.create_user(&json.phone, &json.password) {
            Ok(user_id) => user_id,
            Err(e) => {
                // Гонка: параллельный запрос успел зарегистрировать этот номер.
                // Отдаём 409, а не 500 с внутренней ошибкой SQLite.
                if db
                    .get_user_id_by_phone(&json.phone)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    return Ok(HttpResponse::Conflict()
                        .json(json!({ "error": "Номер уже зарегистрирован" })));
                }
                return Ok(server_error("failed to create user", e));
            }
        },
        None => {
            // Холостое хеширование выравнивает время ответа с веткой входа,
            // иначе по задержке видно, зарегистрирован ли номер.
            let _ = bcrypt::hash(&json.password, bcrypt::DEFAULT_COST);
            return Ok(invalid_credentials());
        }
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

/// Отзывает текущий токен и рвёт WebSocket-соединения этой сессии.
pub async fn logout(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
) -> Result<HttpResponse> {
    if let Some(token) = auth::token_from_request(&req) {
        // user_id нужен до отзыва: после revoke токен уже не разрешается.
        let user_id = db.user_id_by_token(&token).ok().flatten();
        if let Err(e) = db.revoke_token(&token) {
            return Ok(server_error("failed to revoke token", e));
        }
        if let Some(user_id) = user_id {
            state.disconnect_user(user_id);
        }
    }
    Ok(HttpResponse::Ok().json(json!({ "success": true })))
}

/// Список чатов текущего пользователя (пользователь берётся из токена).
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

    let rows = match db.get_user_chats(user_id) {
        Ok(rows) => rows,
        Err(e) => return Ok(server_error("failed to load chats", e)),
    };

    // Заголовок и последнее сообщение уже разрешены одним запросом в db,
    // без N+1 по истории для каждого чата.
    let chats: Vec<ChatItem> = rows
        .into_iter()
        .map(|row| ChatItem {
            id: row.id,
            title: row.title,
            is_group: row.is_group,
            last_message: row.last_message,
            last_timestamp: row.last_timestamp,
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
            let messages: Vec<MessageItem> = rows
                .into_iter()
                .map(
                    |(id, sender_id, text, ts, _out, read, delivered, media_path, media_type, media_size, is_media)| {
                        MessageItem {
                            id,
                            sender_id,
                            text,
                            timestamp: ts,
                            // Одна колонка is_outgoing хранит «как видел отправитель»;
                            // для зрителя исходящее = отправлено им самим.
                            is_outgoing: sender_id == user_id,
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
