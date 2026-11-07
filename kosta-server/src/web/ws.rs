// kosta-server/src/web/ws.rs
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use actix::{Actor, StreamHandler, AsyncContext, Handler, Message, ActorContext};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, error, warn};
use crate::db::Database;
use crate::web::state::WebState;
use crate::metrics::{WS_CONNECTIONS, MESSAGES_SENT, AUTH_SUCCESS};
// Импорт rate limiter
use crate::rate_limiter::{
    check_limit, WS_AUTH_LIMITER, WS_FRAME_LIMITER, WS_MESSAGE_LIMITER, WS_TYPING_LIMITER,
};

/// Максимальная длина текста сообщения в символах (не байтах):
/// кириллица и emoji иначе упирались бы в лимит раньше времени.
const MAX_MESSAGE_CHARS: usize = 4096;

/// Сколько символов мусорного кадра попадает в лог. Полный 64-КиБ фрейм
/// в `warn!` — это лог-амплификация со стороны анонимного клиента.
const MAX_LOGGED_FRAME_CHARS: usize = 200;

/// Обрезает строку для лога по границе символа, не по байту.
fn truncate_for_log(text: &str) -> String {
    let mut truncated: String = text.chars().take(MAX_LOGGED_FRAME_CHARS).collect();
    if text.chars().count() > MAX_LOGGED_FRAME_CHARS {
        truncated.push('…');
    }
    truncated
}

/// Токен может быть отозван (logout в другом окне), поэтому открытый сокет
/// перепроверяем периодически, а не только в момент auth.
const TOKEN_RECHECK_INTERVAL: Duration = Duration::from_secs(30);

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

/// Принудительное отключение соединения: так logout рвёт уже открытые сокеты.
#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect;

// =============================================================================
// Вспомогательные ответы
// =============================================================================

fn send_error(ctx: &mut ws::WebsocketContext<WsActor>, text: &str) {
    let resp = serde_json::json!({
        "type": "error",
        "payload": { "error": text }
    });
    ctx.text(resp.to_string());
}

fn send_auth_failed(ctx: &mut ws::WebsocketContext<WsActor>, text: &str) {
    let resp = serde_json::json!({
        "type": "auth_failed",
        "payload": { "error": text }
    });
    ctx.text(resp.to_string());
}

fn send_auth_success(ctx: &mut ws::WebsocketContext<WsActor>, user_id: i64) {
    let resp = serde_json::json!({
        "type": "auth_success",
        "payload": { "user_id": user_id }
    });
    ctx.text(resp.to_string());
}

// =============================================================================
// WebSocket актор
// =============================================================================

pub struct WsActor {
    pub db: Arc<Database>,
    pub state: WebState,
    pub user_id: Option<i64>,
    pub dialog_id: Option<i64>,
    /// Храним токен, чтобы перепроверять, не отозван ли он.
    pub token: Option<String>,
    /// IP нужен для лимита auth: до авторизации соединение анонимно.
    pub peer_ip: String,
}

impl WsActor {
    /// Сессия ещё жива? До auth проверять нечего.
    fn session_valid(&self) -> bool {
        let Some(user_id) = self.user_id else {
            return true;
        };
        let Some(token) = self.token.as_deref() else {
            return false;
        };
        match self.db.user_id_by_token(token) {
            Ok(found) => found == Some(user_id),
            Err(e) => {
                // Сбой БД — не повод выкидывать пользователя: перепроверка
                // повторится через интервал.
                error!("Failed to verify WebSocket token: {}", e);
                true
            }
        }
    }

    /// Проверяет авторизацию перед действием. Неавторизованным отправляет
    /// ошибку, а отозванный токен — обрывает соединение.
    fn require_auth(&mut self, ctx: &mut ws::WebsocketContext<Self>) -> Option<i64> {
        let Some(user_id) = self.user_id else {
            send_error(ctx, "Not authenticated");
            return None;
        };
        if !self.session_valid() {
            send_auth_failed(ctx, "Сессия завершена");
            ctx.stop();
            return None;
        }
        Some(user_id)
    }
}

impl Actor for WsActor {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        WS_CONNECTIONS.inc();
        info!("WebSocket connection opened");
        ctx.text(r#"{"type":"connected","payload":{}}"#);

        // Периодическая перепроверка токена: logout в другом окне или
        // истечение срока должны закрывать уже открытый сокет.
        ctx.run_interval(TOKEN_RECHECK_INTERVAL, |act, ctx| {
            if !act.session_valid() {
                send_auth_failed(ctx, "Сессия завершена");
                ctx.stop();
            }
        });
    }

    fn stopped(&mut self, ctx: &mut Self::Context) {
        WS_CONNECTIONS.dec();
        // Подписку снимаем всегда, иначе WebState копит адреса мёртвых акторов.
        if let Some(dialog_id) = self.dialog_id.take() {
            self.state.remove_connection(dialog_id, &ctx.address());
        }
        if let Some(user_id) = self.user_id {
            // offline — только когда других соединений пользователя не осталось.
            if self.state.unregister_user(user_id, &ctx.address()) {
                if let Err(e) = self.db.set_user_status(user_id, 1) {
                    error!("Failed to set offline status for user {}: {}", user_id, e);
                }
                self.state.broadcast_status(user_id, 1);
            }
            info!("WebSocket connection closed for user {}", user_id);
        }
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsActor {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                // Дешёвый общий лимит кадров по IP: иначе анонимный клиент
                // может заливать лог мусорными JSON-фреймами (до 64 КиБ).
                if let Err(e) = check_limit(&WS_FRAME_LIMITER, self.peer_ip.clone()) {
                    send_error(ctx, e);
                    return;
                }
                let Ok(json) = serde_json::from_str::<Value>(&text) else {
                    warn!(
                        "Invalid JSON ({} chars): {}",
                        text.chars().count(),
                        truncate_for_log(&text)
                    );
                    return;
                };
                let action = json.get("action").and_then(Value::as_str);
                let payload = json.get("payload").cloned().unwrap_or(Value::Null);

                match action {
                    // ---------- Аутентификация ----------
                    Some("auth") => {
                        // Лимит по IP: до auth соединение анонимно, а каждая
                        // попытка — это запрос в БД с перебором токена.
                        if let Err(e) = check_limit(&WS_AUTH_LIMITER, self.peer_ip.clone()) {
                            send_auth_failed(ctx, e);
                            return;
                        }

                        // user_id из payload больше не принимается — только токен.
                        let token = payload.get("token").and_then(Value::as_str).unwrap_or("");
                        let resolved = if token.is_empty() {
                            None
                        } else {
                            match self.db.user_id_by_token(token) {
                                Ok(user_id) => user_id,
                                Err(e) => {
                                    error!("WebSocket auth database error: {}", e);
                                    None
                                }
                            }
                        };

                        let Some(user_id) = resolved else {
                            send_auth_failed(ctx, "Invalid or expired token");
                            error!("WebSocket auth failed: invalid token");
                            return;
                        };

                        // Сменить пользователя на живом соединении нельзя:
                        // иначе чужая подписка на диалог досталась бы другому аккаунту.
                        if self.user_id.is_some_and(|current| current != user_id) {
                            send_auth_failed(ctx, "Соединение уже авторизовано другим пользователем");
                            error!("WebSocket auth failed: attempted user switch");
                            return;
                        }

                        // Повторная auth тем же пользователем — просто подтверждаем
                        // и не регистрируем соединение второй раз.
                        if self.user_id == Some(user_id) {
                            self.token = Some(token.to_string());
                            send_auth_success(ctx, user_id);
                            info!("WebSocket re-authenticated with user_id={}", user_id);
                            return;
                        }

                        self.user_id = Some(user_id);
                        self.token = Some(token.to_string());
                        // true — только для первого соединения: второе не должно
                        // повторно «включать» пользователя и слать статус.
                        if self.state.register_user(user_id, ctx.address()) {
                            if let Err(e) = self.db.set_user_status(user_id, 0) {
                                error!("Failed to set online status for user {}: {}", user_id, e);
                            }
                            self.state.broadcast_status(user_id, 0);
                        }
                        AUTH_SUCCESS.inc();
                        send_auth_success(ctx, user_id);
                        info!("WebSocket authenticated with user_id={}", user_id);
                    }

                    // ---------- Подписка ----------
                    Some("subscribe") => {
                        let Some(user_id) = self.require_auth(ctx) else {
                            return;
                        };
                        let Some(dialog_id) = payload.get("dialog_id").and_then(Value::as_i64) else {
                            send_error(ctx, "Missing dialog_id");
                            return;
                        };
                        match self.db.get_dialog_participants(dialog_id) {
                            Ok(participants) if participants.contains(&user_id) => {
                                // Одно соединение — одна подписка: старую снимаем,
                                // иначе адрес копится на все диалоги сразу.
                                if let Some(previous) = self.dialog_id.take() {
                                    if previous != dialog_id {
                                        self.state.remove_connection(previous, &ctx.address());
                                    }
                                }
                                self.dialog_id = Some(dialog_id);
                                self.state.add_connection(dialog_id, ctx.address());
                                let resp = serde_json::json!({
                                    "type": "subscribed",
                                    "payload": { "dialog_id": dialog_id }
                                });
                                ctx.text(resp.to_string());
                                info!("WebSocket subscribed to dialog {}", dialog_id);
                            }
                            Ok(_) => {
                                send_error(ctx, "You are not a participant of this dialog");
                            }
                            Err(e) => {
                                error!("Failed to load dialog {} participants: {}", dialog_id, e);
                                send_error(ctx, "Dialog not found");
                            }
                        }
                    }

                    // ---------- Отправка сообщения ----------
                    Some("send_message") => {
                        let Some(user_id) = self.require_auth(ctx) else {
                            return;
                        };
                        let Some(dialog_id) = payload.get("dialog_id").and_then(Value::as_i64) else {
                            send_error(ctx, "Missing dialog_id");
                            return;
                        };
                        let Some(text) = payload.get("text").and_then(Value::as_str) else {
                            send_error(ctx, "Missing text");
                            return;
                        };
                        if text.trim().is_empty() {
                            send_error(ctx, "Message text is empty");
                            return;
                        }
                        if text.chars().count() > MAX_MESSAGE_CHARS {
                            send_error(ctx, "Message text is too long (max 4096 characters)");
                            return;
                        }

                        // ---- Проверка rate limit ----
                        if let Err(e) = check_limit(&WS_MESSAGE_LIMITER, user_id) {
                            send_error(ctx, e);
                            return;
                        }

                        // Проверка участия в диалоге
                        match self.db.get_dialog_participants(dialog_id) {
                            Ok(participants) if participants.contains(&user_id) => {}
                            Ok(_) => {
                                send_error(ctx, "You are not a participant of this dialog");
                                return;
                            }
                            Err(e) => {
                                error!("Failed to load dialog {} participants: {}", dialog_id, e);
                                send_error(ctx, "Dialog not found");
                                return;
                            }
                        }

                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64;

                        match self.db.save_message(dialog_id, user_id, text, now, true, None, None) {
                            Ok(msg_id) => {
                                MESSAGES_SENT.inc();

                                // Один и тот же объект уходит отправителю и остальным,
                                // но `is_outgoing` зависит от зрителя: у отправителя
                                // это его сообщение, у остальных — входящее.
                                let message_payload = |outgoing: bool| {
                                    serde_json::json!({
                                        "type": "new_message",
                                        "payload": {
                                            "id": msg_id,
                                            "dialog_id": dialog_id,
                                            "sender_id": user_id,
                                            "text": text,
                                            "timestamp": now,
                                            "is_outgoing": outgoing,
                                            "read": false,
                                            "delivered": false,
                                        }
                                    })
                                };
                                let own_str = message_payload(true).to_string();
                                let others_str = message_payload(false).to_string();

                                ctx.text(own_str);
                                let self_addr = ctx.address();
                                self.state.broadcast_to_dialog(dialog_id, &others_str, Some(&self_addr));

                                info!("Message {} broadcasted to dialog {}", msg_id, dialog_id);
                            }
                            Err(e) => {
                                error!("Failed to save message: {}", e);
                                send_error(ctx, "Failed to save message");
                            }
                        }
                    }

                    // ---------- Индикатор набора ----------
                    Some("typing") => {
                        let Some(user_id) = self.require_auth(ctx) else {
                            return;
                        };
                        let Some(dialog_id) = payload.get("dialog_id").and_then(Value::as_i64) else {
                            send_error(ctx, "Missing dialog_id");
                            return;
                        };
                        let Some(typing) = payload.get("typing").and_then(Value::as_bool) else {
                            send_error(ctx, "Missing typing");
                            return;
                        };

                        // Отдельный лимит: typing дешёвый, но рассылается всем
                        // подписчикам, поэтому поток с одного аккаунта ограничен.
                        if let Err(e) = check_limit(&WS_TYPING_LIMITER, user_id) {
                            send_error(ctx, e);
                            return;
                        }

                        // Рассылать индикатор можно только в свой диалог.
                        match self.db.get_dialog_participants(dialog_id) {
                            Ok(participants) if participants.contains(&user_id) => {}
                            Ok(_) => {
                                send_error(ctx, "You are not a participant of this dialog");
                                return;
                            }
                            Err(e) => {
                                error!("Failed to load dialog {} participants: {}", dialog_id, e);
                                send_error(ctx, "Dialog not found");
                                return;
                            }
                        }

                        let event = serde_json::json!({
                            "type": "typing",
                            "payload": {
                                "dialog_id": dialog_id,
                                "user_id": user_id,
                                "typing": typing,
                            }
                        })
                        .to_string();
                        // Себе не эхо: клиент и так знает, что пользователь печатает.
                        let self_addr = ctx.address();
                        self.state
                            .broadcast_to_dialog(dialog_id, &event, Some(&self_addr));
                    }

                    // ---------- Остальные действия (read) ----------
                    _ => {
                        warn!("Unknown WebSocket action: {:?}", action);
                    }
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

impl Handler<Disconnect> for WsActor {
    type Result = ();

    fn handle(&mut self, _msg: Disconnect, ctx: &mut Self::Context) {
        // Реестр уже очищен disconnect_user, поэтому offline ставим здесь:
        // остановка актора больше не увидит себя последним соединением.
        if let Some(user_id) = self.user_id {
            if let Err(e) = self.db.set_user_status(user_id, 1) {
                error!("Failed to set offline status for user {}: {}", user_id, e);
            }
        }
        send_auth_failed(ctx, "Сессия завершена");
        ctx.stop();
    }
}

// =============================================================================
// HTTP эндпоинт для WebSocket
// =============================================================================

pub async fn websocket_handler(
    req: HttpRequest,
    stream: web::Payload,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
) -> Result<HttpResponse, actix_web::Error> {
    // IP нужен для лимита auth; за прокси здесь будет адрес прокси,
    // но WebSocket-хендшейк в этом проекте прямой.
    let peer_ip = req
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let ws = WsActor {
        db: db.get_ref().clone(),
        state: state.get_ref().clone(),
        user_id: None,
        dialog_id: None,
        token: None,
        peer_ip,
    };
    ws::start(ws, &req, stream)
}
