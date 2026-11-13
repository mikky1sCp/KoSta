// kosta-server/src/web/handlers/auth.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::rate_limiter::{check_limit, LOGIN_LIMITER};
use crate::web::auth;
use crate::web::client_addr::client_ip;
use crate::web::models::{LoginRequest, LoginResponse};
use crate::web::state::WebState;

use super::{
    bad_request, invalid_credentials, rate_limited, server_error, valid_password, valid_phone,
};

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

    // auth_key_id нужен только как справка о существующей MTProto 2.0-сессии.
    let auth_key_id = db.get_mtproto_auth_key_id_for_user(user_id).ok().flatten();

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
