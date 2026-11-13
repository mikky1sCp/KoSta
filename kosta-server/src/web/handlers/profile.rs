// kosta-server/src/web/handlers/profile.rs
use actix_web::{http::header, web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::media;
use crate::rate_limiter::{check_limit, HTTP_LIMITER};
use crate::web::auth;
use crate::web::client_addr::client_ip;
use crate::web::models::{LookupQuery, LookupResponse, MeResponse};

use super::{bad_request, not_found, path_id, rate_limited, server_error, MAX_DISPLAY_NAME_CHARS};

/// Профиль текущего пользователя.
///
/// `is_online` берётся из `user_status` (status == 0), а не из реестра
/// WebSocket-соединений: отсутствие строки в таблице — offline.
pub async fn get_me(req: HttpRequest, db: web::Data<Arc<Database>>) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let profile = match db.get_user_profile(user_id) {
        Ok(profile) => profile,
        Err(e) => return Ok(server_error("failed to load profile", e)),
    };
    let Some(profile) = profile else {
        return Ok(not_found("user not found"));
    };

    Ok(HttpResponse::Ok().json(MeResponse {
        user_id,
        phone: profile.phone,
        display_name: profile.display_name,
        has_avatar: profile.avatar_key.is_some(),
        is_online: profile.is_online,
    }))
}

/// Тело `PATCH /api/me`.
#[derive(Debug, serde::Deserialize)]
pub struct UpdateProfileBody {
    pub display_name: String,
}

/// Правка своего профиля: `PATCH /api/me`.
pub async fn update_me(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    body: web::Json<UpdateProfileBody>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }
    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let name = body.display_name.trim();
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Ok(bad_request("display name is too long (max 64 characters)"));
    }
    if let Err(e) = db.update_display_name(user_id, name) {
        return Ok(server_error("failed to update profile", e));
    }

    let profile = match db.get_user_profile(user_id) {
        Ok(Some(profile)) => profile,
        Ok(None) => return Ok(not_found("user not found")),
        Err(e) => return Ok(server_error("failed to load profile", e)),
    };

    Ok(HttpResponse::Ok().json(MeResponse {
        user_id,
        phone: profile.phone,
        display_name: profile.display_name,
        has_avatar: profile.avatar_key.is_some(),
        is_online: profile.is_online,
    }))
}

/// Загрузка аватара: `POST /api/me/avatar` (сырое тело картинки).
pub async fn upload_avatar(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    body: web::Bytes,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }
    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let mime = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if body.is_empty() {
        return Ok(bad_request("empty image"));
    }

    let key = match media::save_avatar(user_id, &body, &mime) {
        Ok(key) => key,
        Err(e) => {
            let message = e.to_string();
            if message.contains("unsupported avatar type") {
                return Ok(HttpResponse::UnsupportedMediaType()
                    .json(json!({ "error": "only PNG, JPEG and WebP images are allowed" })));
            }
            if message.contains("too large") {
                return Ok(
                    HttpResponse::PayloadTooLarge().json(json!({ "error": "image is too large" }))
                );
            }
            return Ok(server_error("failed to store avatar", e));
        }
    };

    match db.set_avatar_key(user_id, &key) {
        Ok(previous) => {
            if let Some(previous) = previous {
                media::delete_avatar(&previous);
            }
            Ok(HttpResponse::Ok().json(json!({ "ok": true, "has_avatar": true })))
        }
        Err(e) => {
            media::delete_avatar(&key);
            Ok(server_error("failed to save avatar", e))
        }
    }
}

/// Аватар пользователя: `GET /api/avatars/{user_id}`.
///
/// Без токена: картинка профиля нужна в тегах `<img>`, а браузер не отправит
/// заголовок Authorization. Аватар и так виден всем, кто состоит с человеком
/// в общем диалоге; перебор по id ограничен общим лимитом HTTP.
pub async fn get_avatar(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }
    let user_id = match path_id(&path, "id пользователя") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let key = match db.get_avatar_key(user_id) {
        Ok(Some(key)) => key,
        Ok(None) => return Ok(not_found("avatar not found")),
        Err(e) => return Ok(server_error("failed to load avatar", e)),
    };
    let bytes = match media::read_avatar(&key) {
        Ok(bytes) => bytes,
        Err(e) => return Ok(server_error("failed to read avatar", e)),
    };

    let mime = match key.rsplit_once('.').map(|(_, ext)| ext) {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };

    Ok(HttpResponse::Ok()
        .content_type(mime)
        .insert_header((header::CACHE_CONTROL, "public, max-age=60"))
        .body(bytes))
}

/// Точный поиск пользователя по номеру — для «новый чат» и добавления
/// участника. Доступно только авторизованным.
pub async fn lookup_user(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    query: web::Query<LookupQuery>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    if let Err(resp) = auth::authenticated_user(&req, &db) {
        return Ok(resp);
    }

    let Some(phone) = query.phone.as_deref().filter(|phone| !phone.is_empty()) else {
        return Ok(bad_request("Параметр phone обязателен"));
    };

    let found = match db.find_user_by_phone(phone) {
        Ok(found) => found,
        Err(e) => return Ok(server_error("failed to lookup user", e)),
    };

    match found {
        Some((user_id, phone)) => Ok(HttpResponse::Ok().json(LookupResponse { user_id, phone })),
        None => Ok(not_found("user not found")),
    }
}
