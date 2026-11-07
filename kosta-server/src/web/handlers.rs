// kosta-server/src/web/handlers.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use anyhow::Result as AnyhowResult;

use crate::db::{ChatRow, Database, ParticipantRow};
use crate::rate_limiter::{check_limit, HTTP_LIMITER, LOGIN_LIMITER};
use crate::web::auth;
use crate::web::models::*;
use crate::web::state::WebState;

/// Максимум участников группы, включая создателя (как в TL-обработчиках).
const MAX_GROUP_PARTICIPANTS: usize = 200;
/// Создатель добавляется сам, поэтому в теле создания группы — до 199 номеров.
const MAX_GROUP_INVITEES: usize = MAX_GROUP_PARTICIPANTS - 1;
/// Максимальная длина названия группы в символах (не байтах).
const MAX_GROUP_TITLE_CHARS: usize = 128;

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

fn not_found(message: &str) -> HttpResponse {
    HttpResponse::NotFound().json(json!({ "error": message }))
}

fn forbidden(message: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(json!({ "error": message }))
}

/// Числовой id из пути. Разбираем вручную, чтобы ошибка была в общем
/// JSON-формате: экстрактор `web::Path<i64>` на мусоре отдаёт текстовый 400.
#[allow(clippy::result_large_err)] // готовый HttpResponse — самый прямой путь
fn path_id(raw: &str, what: &str) -> std::result::Result<i64, HttpResponse> {
    raw.parse::<i64>()
        .map_err(|_| bad_request(&format!("Некорректный {}", what)))
}

/// Участники существующего диалога или готовый ответ: 404 — диалога нет,
/// 500 — сбой БД.
#[allow(clippy::result_large_err)] // как и в auth.rs: возвращаем готовый ответ
fn dialog_participants(
    db: &Database,
    dialog_id: i64,
) -> std::result::Result<Vec<i64>, HttpResponse> {
    match db.get_dialog_participants(dialog_id) {
        Ok(participants) if participants.is_empty() => Err(not_found("dialog not found")),
        Ok(participants) => Ok(participants),
        Err(e) => Err(server_error("failed to load dialog", e)),
    }
}

/// Карточка чата из строки БД. У группы собеседника нет, поэтому
/// `is_online` в JSON не попадает (None), а не приходит как false.
fn chat_item(row: ChatRow) -> ChatItem {
    ChatItem {
        id: row.id,
        title: row.title,
        is_group: row.is_group,
        last_message: row.last_message,
        last_timestamp: row.last_timestamp,
        unread_count: row.unread_count,
        other_user_id: row.other_user_id,
        is_online: (!row.is_group).then_some(row.is_online),
    }
}

fn participant_item(row: ParticipantRow) -> ParticipantItem {
    ParticipantItem {
        user_id: row.user_id,
        phone: row.phone,
        is_admin: row.is_admin,
        is_online: row.is_online,
    }
}

/// Собирает `ChatDetails` глазами зрителя. `None` — карточки нет (зритель
/// не участник или диалог успели удалить).
fn build_chat_details(
    db: &Database,
    viewer_id: i64,
    dialog_id: i64,
) -> AnyhowResult<Option<ChatDetails>> {
    let Some(row) = db.get_chat_row(viewer_id, dialog_id)? else {
        return Ok(None);
    };
    let participants = db.get_dialog_participants_detailed(dialog_id)?;
    let is_admin = db.is_dialog_admin(dialog_id, viewer_id)?;
    Ok(Some(ChatDetails {
        chat: chat_item(row),
        participants: participants.into_iter().map(participant_item).collect(),
        is_admin,
    }))
}

/// Готовый 200-ответ с деталями диалога (или 404/500, если их не собрать).
fn chat_details_response(db: &Database, viewer_id: i64, dialog_id: i64) -> HttpResponse {
    match build_chat_details(db, viewer_id, dialog_id) {
        Ok(Some(details)) => HttpResponse::Ok().json(details),
        Ok(None) => not_found("dialog not found"),
        Err(e) => server_error("failed to load chat details", e),
    }
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
    // без N+1 по истории для каждого чата. Непрочитанные и статус собеседника
    // считаются там же.
    let chats: Vec<ChatItem> = rows.into_iter().map(chat_item).collect();

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
    let Some((phone, is_online)) = profile else {
        return Ok(not_found("user not found"));
    };

    Ok(HttpResponse::Ok().json(MeResponse {
        user_id,
        phone,
        is_online,
    }))
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

/// Детали диалога: карточка, участники, роль текущего пользователя.
/// Доступно только участникам — и для групп, и для личных диалогов.
pub async fn get_chat(
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

    Ok(chat_details_response(&db, user_id, dialog_id))
}

/// Находит или создаёт личный диалог с пользователем `phone`.
pub async fn create_private_chat(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    json: web::Json<CreatePrivateChatRequest>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let phone = json.phone.as_str();
    if !valid_phone(phone) {
        return Ok(bad_request("Некорректный номер телефона"));
    }

    let target = match db.find_user_by_phone(phone) {
        Ok(target) => target,
        Err(e) => return Ok(server_error("failed to lookup user", e)),
    };
    let Some((target_id, _)) = target else {
        return Ok(not_found("user not found"));
    };
    if target_id == user_id {
        return Ok(bad_request("Нельзя создать диалог с самим собой"));
    }

    // create_private_dialog идемпотентен: существующий диалог переиспользуется.
    let dialog_id = match db.create_private_dialog(user_id, target_id) {
        Ok(id) => id,
        Err(e) => return Ok(server_error("failed to create private dialog", e)),
    };

    Ok(chat_details_response(&db, user_id, dialog_id))
}

/// Создаёт группу. Создатель становится администратором.
pub async fn create_group_chat(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    json: web::Json<CreateGroupChatRequest>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let title = json.title.trim();
    if title.is_empty() || title.chars().count() > MAX_GROUP_TITLE_CHARS {
        return Ok(bad_request("Название группы должно быть от 1 до 128 символов"));
    }
    if json.phones.len() > MAX_GROUP_INVITEES {
        return Ok(bad_request("Слишком много участников: максимум 199 номеров"));
    }

    // Дедупликация по номеру: повтор номера в запросе — не ошибка.
    let mut seen: Vec<&str> = Vec::new();
    let mut participants: Vec<i64> = Vec::new();
    for phone in &json.phones {
        if !valid_phone(phone) {
            return Ok(bad_request("Некорректный номер телефона"));
        }
        if seen.contains(&phone.as_str()) {
            continue;
        }
        seen.push(phone.as_str());

        let found = match db.find_user_by_phone(phone) {
            Ok(found) => found,
            Err(e) => return Ok(server_error("failed to lookup user", e)),
        };
        let Some((id, _)) = found else {
            // Указываем конкретный номер: иначе фронтенду нечего подсветить.
            return Ok(not_found(&format!("user not found: {}", phone)));
        };
        // Создатель станет администратором сам, повторно его не добавляем.
        if id == user_id || participants.contains(&id) {
            continue;
        }
        participants.push(id);
    }

    let dialog_id = match db.create_group_dialog(title, user_id, &participants) {
        Ok(id) => id,
        Err(e) => return Ok(server_error("failed to create group", e)),
    };

    Ok(chat_details_response(&db, user_id, dialog_id))
}

/// Переименование группы. Только администратор; личные диалоги не меняются.
pub async fn rename_chat(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    path: web::Path<String>,
    json: web::Json<RenameChatRequest>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let dialog_id = match path_id(&path, "id диалога") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let typ = match db.get_dialog_type(dialog_id) {
        Ok(Some(typ)) => typ,
        Ok(None) => return Ok(not_found("dialog not found")),
        Err(e) => return Ok(server_error("failed to load dialog", e)),
    };
    let participants = match dialog_participants(&db, dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }
    if typ != 1 {
        return Ok(bad_request("Личный диалог нельзя переименовать"));
    }

    let is_admin = match db.is_dialog_admin(dialog_id, user_id) {
        Ok(is_admin) => is_admin,
        Err(e) => return Ok(server_error("failed to check dialog role", e)),
    };
    if !is_admin {
        return Ok(forbidden("Только администратор группы может переименовать её"));
    }

    let title = json.title.trim();
    if title.is_empty() || title.chars().count() > MAX_GROUP_TITLE_CHARS {
        return Ok(bad_request("Название группы должно быть от 1 до 128 символов"));
    }

    if let Err(e) = db.rename_dialog(dialog_id, title) {
        return Ok(server_error("failed to rename chat", e));
    }

    Ok(chat_details_response(&db, user_id, dialog_id))
}

/// Добавляет участника в группу. Только администратор; уже добавленный
/// участник — не ошибка (идемпотентно).
pub async fn add_participant(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    path: web::Path<String>,
    json: web::Json<AddParticipantRequest>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let dialog_id = match path_id(&path, "id диалога") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let typ = match db.get_dialog_type(dialog_id) {
        Ok(Some(typ)) => typ,
        Ok(None) => return Ok(not_found("dialog not found")),
        Err(e) => return Ok(server_error("failed to load dialog", e)),
    };
    let participants = match dialog_participants(&db, dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }
    if typ != 1 {
        return Ok(bad_request("Участников можно добавлять только в группу"));
    }

    let is_admin = match db.is_dialog_admin(dialog_id, user_id) {
        Ok(is_admin) => is_admin,
        Err(e) => return Ok(server_error("failed to check dialog role", e)),
    };
    if !is_admin {
        return Ok(forbidden("Только администратор группы может добавлять участников"));
    }

    let phone = json.phone.as_str();
    if !valid_phone(phone) {
        return Ok(bad_request("Некорректный номер телефона"));
    }
    let found = match db.find_user_by_phone(phone) {
        Ok(found) => found,
        Err(e) => return Ok(server_error("failed to lookup user", e)),
    };
    let Some((target_id, _)) = found else {
        return Ok(not_found("user not found"));
    };

    // Повторное добавление — успех, а не 409: фронтенд может не знать,
    // что участник уже в группе.
    if participants.contains(&target_id) {
        return Ok(chat_details_response(&db, user_id, dialog_id));
    }
    if participants.len() >= MAX_GROUP_PARTICIPANTS {
        return Ok(bad_request("В группе уже максимум участников (200)"));
    }

    if let Err(e) = db.add_participant(dialog_id, target_id) {
        return Ok(server_error("failed to add participant", e));
    }

    Ok(chat_details_response(&db, user_id, dialog_id))
}

/// Удаление участника: админ удаляет любого, участник — себя (выход).
/// После выхода без администратора назначается новый, пустой диалог удаляется.
pub async fn remove_participant(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    if let Err(e) = check_limit(&HTTP_LIMITER, client_ip(&req)) {
        return Ok(rate_limited(e));
    }

    let user_id = match auth::authenticated_user(&req, &db) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let dialog_id = match path_id(&path.0, "id диалога") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let target_id = match path_id(&path.1, "id пользователя") {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let typ = match db.get_dialog_type(dialog_id) {
        Ok(Some(typ)) => typ,
        Ok(None) => return Ok(not_found("dialog not found")),
        Err(e) => return Ok(server_error("failed to load dialog", e)),
    };
    let participants = match dialog_participants(&db, dialog_id) {
        Ok(participants) => participants,
        Err(resp) => return Ok(resp),
    };
    if !participants.contains(&user_id) {
        return Ok(forbidden("not a participant"));
    }
    if typ != 1 {
        return Ok(bad_request("Личный диалог нельзя изменить"));
    }

    if target_id != user_id {
        let is_admin = match db.is_dialog_admin(dialog_id, user_id) {
            Ok(is_admin) => is_admin,
            Err(e) => return Ok(server_error("failed to check dialog role", e)),
        };
        if !is_admin {
            return Ok(forbidden("Только администратор может удалить участника"));
        }
        if !participants.contains(&target_id) {
            return Ok(not_found("participant not found"));
        }
    }

    if let Err(e) = db.remove_participant(dialog_id, target_id) {
        return Ok(server_error("failed to remove participant", e));
    }
    // Если ушёл последний админ — повышаем первого оставшегося участника.
    if let Err(e) = db.ensure_group_has_admin(dialog_id) {
        return Ok(server_error("failed to reassign group admin", e));
    }
    // Пустой диалог (и его сообщения) больше не нужен.
    let deleted = match db.delete_empty_dialog(dialog_id) {
        Ok(deleted) => deleted,
        Err(e) => return Ok(server_error("failed to delete empty dialog", e)),
    };

    Ok(HttpResponse::Ok().json(json!({ "deleted": deleted })))
}

/// Помечает входящие сообщения диалога прочитанными и сообщает об этом
/// подписчикам диалога через WebSocket.
pub async fn mark_chat_read(
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

    let marked = match db.mark_dialog_read(dialog_id, user_id) {
        Ok(marked) => marked,
        Err(e) => return Ok(server_error("failed to mark dialog read", e)),
    };

    // Событие получают все подписанные соединения диалога: другие вкладки
    // читателя тоже должны обновить счётчики непрочитанного.
    let event = json!({
        "type": "messages_read",
        "payload": { "dialog_id": dialog_id, "reader_id": user_id }
    })
    .to_string();
    state.broadcast_to_dialog(dialog_id, &event, None);

    Ok(HttpResponse::Ok().json(json!({ "marked": marked })))
}
