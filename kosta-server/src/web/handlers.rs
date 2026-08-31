use actix_web::{web, HttpResponse, Result, HttpRequest};
use serde_json::json;
use crate::db::Database;
use crate::web::models::*;
use std::sync::Arc;
use crate::rate_limiter::{HTTP_LIMITER, check_limit};

pub async fn login(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    json: web::Json<LoginRequest>,
) -> Result<HttpResponse> {
    let ip = req.connection_info().peer_addr().unwrap_or("unknown").to_string();
    if let Err(e) = check_limit(&HTTP_LIMITER, ip) {
        return Ok(HttpResponse::TooManyRequests().json(LoginResponse {
            success: false,
            auth_key_id: None,
            user_id: None,
            error: Some(e.to_string()),
        }));
    }

    match db.authenticate(&json.phone, &json.password) {
        Ok(Some(user_id)) => {
            let auth_key_id = match db.get_full_session_for_user(user_id) {
                Ok(Some(session)) => Some(session.auth_key_id),
                _ => None,
            };
            Ok(HttpResponse::Ok().json(LoginResponse {
                success: true,
                auth_key_id,
                user_id: Some(user_id),
                error: None,
            }))
        }
        Ok(None) => {
            match db.create_user(&json.phone, &json.password) {
                Ok(user_id) => {
                    Ok(HttpResponse::Ok().json(LoginResponse {
                        success: true,
                        auth_key_id: None,
                        user_id: Some(user_id),
                        error: None,
                    }))
                }
                Err(e) => Ok(HttpResponse::InternalServerError().json(LoginResponse {
                    success: false,
                    auth_key_id: None,
                    user_id: None,
                    error: Some(format!("Failed to create user: {}", e)),
                })),
            }
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(LoginResponse {
            success: false,
            auth_key_id: None,
            user_id: None,
            error: Some(format!("Database error: {}", e)),
        })),
    }
}

pub async fn get_chats(
    req: HttpRequest,
    _db: web::Data<Arc<Database>>,
) -> Result<HttpResponse> {
    let ip = req.connection_info().peer_addr().unwrap_or("unknown").to_string();
    if let Err(e) = check_limit(&HTTP_LIMITER, ip) {
        return Ok(HttpResponse::TooManyRequests().body(e));
    }

    let chats = vec![
        ChatItem {
            id: 1,
            title: "General".to_string(),
            is_group: false,
            last_message: Some("Hello".to_string()),
            last_timestamp: Some(1234567890),
        }
    ];
    Ok(HttpResponse::Ok().json(chats))
}

pub async fn get_history(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    query: web::Query<GetHistoryQuery>,
) -> Result<HttpResponse> {
    let ip = req.connection_info().peer_addr().unwrap_or("unknown").to_string();
    if let Err(e) = check_limit(&HTTP_LIMITER, ip) {
        return Ok(HttpResponse::TooManyRequests().body(e));
    }

    let chat_id = query.chat_id;
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);

    match db.get_history(chat_id, offset, limit) {
        Ok(rows) => {
            let messages: Vec<MessageItem> = rows
                .into_iter()
                .map(|(id, sender_id, text, ts, out, read, delivered, media_path, media_type, media_size, is_media)| {
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
                })
                .collect();
            Ok(HttpResponse::Ok().json(messages))
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({ "error": e.to_string() }))),
    }
}