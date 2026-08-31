// kosta-server/src/web/ws.rs
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use actix::{Actor, StreamHandler, AsyncContext, Handler, Message, ActorContext};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Instant, Duration};
use tracing::{info, error, warn};
use crate::db::Database;
use crate::session_cache::SessionCache;
use crate::web::state::WebState;
use crate::metrics::{WS_CONNECTIONS, MESSAGES_SENT, AUTH_SUCCESS};
// Импорт rate limiter
use crate::rate_limiter::{WS_MESSAGE_LIMITER, check_limit};

// =============================================================================
// Сообщения для отправки через WebSocket
// =============================================================================

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct SendMessage(pub String);

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct UserStatusUpdate {
    pub user_id: i64,
    pub status: i32,
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct TypingNotification {
    pub dialog_id: i64,
    pub user_id: i64,
    pub typing: bool,
}

// =============================================================================
// WebSocket актор
// =============================================================================

pub struct WsActor {
    pub db: Arc<Database>,
    pub cache: Arc<SessionCache>,
    pub state: WebState,
    pub user_id: Option<i64>,
    pub dialog_id: Option<i64>,
    pub last_typing_ts: Instant,
}

impl Actor for WsActor {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        WS_CONNECTIONS.inc();
        info!("WebSocket connection opened");
        ctx.text(r#"{"type":"connected","payload":{}}"#);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        WS_CONNECTIONS.dec();
        if let Some(user_id) = self.user_id {
            if let Err(e) = self.db.set_user_status(user_id, 1) {
                error!("Failed to set offline status for user {}: {}", user_id, e);
            }
            info!("WebSocket connection closed for user {}", user_id);
        }
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsActor {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                    let action = json.get("action").and_then(Value::as_str);
                    let payload = json.get("payload").cloned().unwrap_or(Value::Null);

                    match action {
                        // ---------- Аутентификация ----------
                        Some("auth") => {
                            if let Some(user_id) = payload.get("user_id").and_then(Value::as_i64) {
                                if let Ok(Some((_status, _last_seen))) = self.db.get_user_status(user_id) {
                                    self.user_id = Some(user_id);
                                    if let Err(e) = self.db.set_user_status(user_id, 0) {
                                        error!("Failed to set online status for user {}: {}", user_id, e);
                                    }
                                    self.state.broadcast_status(user_id, 0);
                                    AUTH_SUCCESS.inc();

                                    let resp = serde_json::json!({
                                        "type": "auth_success",
                                        "payload": { "user_id": user_id }
                                    });
                                    ctx.text(resp.to_string());
                                    info!("WebSocket authenticated with user_id={}", user_id);
                                } else {
                                    let resp = serde_json::json!({
                                        "type": "auth_failed",
                                        "payload": { "error": "Invalid user_id" }
                                    });
                                    ctx.text(resp.to_string());
                                    error!("WebSocket auth failed: invalid user_id={}", user_id);
                                }
                            }
                        }

                        // ---------- Подписка ----------
                        Some("subscribe") => {
                            if let Some(dialog_id) = payload.get("dialog_id").and_then(Value::as_i64) {
                                if let Ok(participants) = self.db.get_dialog_participants(dialog_id) {
                                    if let Some(user_id) = self.user_id {
                                        if participants.contains(&user_id) {
                                            self.dialog_id = Some(dialog_id);
                                            let addr = ctx.address();
                                            self.state.add_connection(dialog_id, addr);
                                            let resp = serde_json::json!({
                                                "type": "subscribed",
                                                "payload": { "dialog_id": dialog_id }
                                            });
                                            ctx.text(resp.to_string());
                                            info!("WebSocket subscribed to dialog {}", dialog_id);
                                        } else {
                                            let resp = serde_json::json!({
                                                "type": "error",
                                                "payload": { "error": "You are not a participant of this dialog" }
                                            });
                                            ctx.text(resp.to_string());
                                        }
                                    }
                                } else {
                                    let resp = serde_json::json!({
                                        "type": "error",
                                        "payload": { "error": "Dialog not found" }
                                    });
                                    ctx.text(resp.to_string());
                                }
                            }
                        }

                        // ---------- Отправка сообщения ----------
                        Some("send_message") => {
                            if let (Some(dialog_id), Some(text), Some(user_id)) = (
                                payload.get("dialog_id").and_then(Value::as_i64),
                                payload.get("text").and_then(Value::as_str),
                                self.user_id,
                            ) {
                                // ---- Проверка rate limit ----
                                if let Err(e) = check_limit(&WS_MESSAGE_LIMITER, user_id) {
                                    let resp = serde_json::json!({
                                        "type": "error",
                                        "payload": { "error": e }
                                    });
                                    ctx.text(resp.to_string());
                                    return;
                                }

                                // Проверка участия в диалоге
                                if let Ok(participants) = self.db.get_dialog_participants(dialog_id) {
                                    if !participants.contains(&user_id) {
                                        let resp = serde_json::json!({
                                            "type": "error",
                                            "payload": { "error": "You are not a participant of this dialog" }
                                        });
                                        ctx.text(resp.to_string());
                                        return;
                                    }
                                } else {
                                    let resp = serde_json::json!({
                                        "type": "error",
                                        "payload": { "error": "Dialog not found" }
                                    });
                                    ctx.text(resp.to_string());
                                    return;
                                }

                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs() as i64;

                                match self.db.save_message(dialog_id, user_id, text, now, true, None, None) {
                                    Ok(msg_id) => {
                                        MESSAGES_SENT.inc();

                                        let msg_data = serde_json::json!({
                                            "type": "new_message",
                                            "payload": {
                                                "id": msg_id,
                                                "dialog_id": dialog_id,
                                                "sender_id": user_id,
                                                "text": text,
                                                "timestamp": now,
                                                "is_outgoing": true,
                                                "read": false,
                                                "delivered": false,
                                            }
                                        });
                                        let msg_str = msg_data.to_string();

                                        ctx.text(msg_str.clone());
                                        self.state.broadcast_to_dialog(dialog_id, &msg_str, Some(user_id));

                                        info!("Message {} broadcasted to dialog {}", msg_id, dialog_id);
                                    }
                                    Err(e) => {
                                        error!("Failed to save message: {}", e);
                                        let resp = serde_json::json!({
                                            "type": "error",
                                            "payload": { "error": "Failed to save message" }
                                        });
                                        ctx.text(resp.to_string());
                                    }
                                }
                            } else {
                                let resp = serde_json::json!({
                                    "type": "error",
                                    "payload": { "error": "Missing dialog_id, text, or not authenticated" }
                                });
                                ctx.text(resp.to_string());
                            }
                        }

                        // ---------- Остальные действия (typing, read) ----------
                        _ => {
                            warn!("Unknown WebSocket action: {:?}", action);
                        }
                    }
                } else {
                    warn!("Invalid JSON: {}", text);
                }
            }
            Ok(ws::Message::Binary(_)) => {
                warn!("Binary message received, ignoring");
            }
            Ok(ws::Message::Ping(_)) => {
                ctx.pong(&[]);
            }
            Ok(ws::Message::Pong(_)) => {}
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => {
                ctx.stop();
            }
        }
    }
}

// =============================================================================
// Обработчики внутренних сообщений (для рассылки)
// =============================================================================

impl Handler<SendMessage> for WsActor {
    type Result = ();

    fn handle(&mut self, msg: SendMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

impl Handler<UserStatusUpdate> for WsActor {
    type Result = ();

    fn handle(&mut self, msg: UserStatusUpdate, ctx: &mut Self::Context) {
        let status_msg = serde_json::json!({
            "type": "user_status",
            "payload": {
                "user_id": msg.user_id,
                "status": msg.status,
            }
        });
        ctx.text(status_msg.to_string());
    }
}

impl Handler<TypingNotification> for WsActor {
    type Result = ();

    fn handle(&mut self, msg: TypingNotification, ctx: &mut Self::Context) {
        let typing_msg = serde_json::json!({
            "type": "typing",
            "payload": {
                "dialog_id": msg.dialog_id,
                "user_id": msg.user_id,
                "typing": msg.typing,
            }
        });
        ctx.text(typing_msg.to_string());
    }
}

// =============================================================================
// HTTP эндпоинт для WebSocket
// =============================================================================

pub async fn websocket_handler(
    req: HttpRequest,
    stream: web::Payload,
    db: web::Data<Arc<Database>>,
    cache: web::Data<Arc<SessionCache>>,
    state: web::Data<WebState>,
) -> Result<HttpResponse, actix_web::Error> {
    let ws = WsActor {
        db: db.get_ref().clone(),
        cache: cache.get_ref().clone(),
        state: state.get_ref().clone(),
        user_id: None,
        dialog_id: None,
        last_typing_ts: Instant::now(),
    };
    ws::start(ws, &req, stream)
}