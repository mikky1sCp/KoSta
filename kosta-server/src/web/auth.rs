// kosta-server/src/web/auth.rs
//! Аутентификация HTTP- и WebSocket-запросов по сессионному токену.

use actix_web::{HttpRequest, HttpResponse};
use serde_json::json;

use crate::db::Database;

/// Проверяет токен и возвращает пользователя либо готовый 401-ответ.
/// `HttpResponse` в Err-ветке крупный, но это самый прямой способ вернуть
/// хендлеру готовый ответ без дублирования кода.
#[allow(clippy::result_large_err)]
pub fn authenticated_user(req: &HttpRequest, db: &Database) -> Result<i64, HttpResponse> {
    match token_from_request(req) {
        Some(token) => db
            .user_id_by_token(&token)
            .map_err(|_| unauthorized())?
            .ok_or_else(unauthorized),
        None => Err(unauthorized()),
    }
}

/// Возвращает токен из запроса (нужен для logout).
pub fn token_from_request(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Ответ для неаутентифицированного запроса.
pub fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(json!({ "error": "unauthorized" }))
}
