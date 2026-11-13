// kosta-server/src/web/handlers/chats.rs
use actix_web::{web, HttpRequest, HttpResponse, Result};
use serde_json::json;
use std::sync::Arc;

use crate::db::Database;
use crate::media;
use crate::rate_limiter::{check_limit, HTTP_LIMITER};
use crate::web::auth;
use crate::web::client_addr::client_ip;
use crate::web::models::{
    AddParticipantRequest, ChatItem, ChatStateRequest, CreateGroupChatRequest,
    CreatePrivateChatRequest, RenameChatRequest,
};
use crate::web::state::WebState;

use super::messages::message_item_from_row;
use super::{
    bad_request, chat_details_response, chat_item, dialog_participants, forbidden, not_found,
    now_secs, path_id, rate_limited, server_error, valid_phone, MAX_GROUP_INVITEES,
    MAX_GROUP_PARTICIPANTS, MAX_GROUP_TITLE_CHARS,
};

/// Список чатов текущего пользователя (пользователь берётся из токена).
pub async fn get_chats(req: HttpRequest, db: web::Data<Arc<Database>>) -> Result<HttpResponse> {
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

    let phone = json.phone.trim();
    let username = json.username.trim();

    // Достаточно телефона или username: второй способ нужен, когда человек
    // найден через поиск людей и его номер не показывается.
    let target_id = if !phone.is_empty() {
        if !valid_phone(phone) {
            return Ok(bad_request("Некорректный номер телефона"));
        }
        match db.find_user_by_phone(phone) {
            Ok(Some((target_id, _))) => target_id,
            Ok(None) => return Ok(not_found("user not found")),
            Err(e) => return Ok(server_error("failed to lookup user", e)),
        }
    } else if !username.is_empty() {
        match db.find_user_by_username(username) {
            Ok(Some(target_id)) => target_id,
            Ok(None) => return Ok(not_found("user not found")),
            Err(e) => return Ok(server_error("failed to lookup user", e)),
        }
    } else {
        return Ok(bad_request("Укажите phone или username"));
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
        return Ok(bad_request(
            "Название группы должно быть от 1 до 128 символов",
        ));
    }
    if json.phones.len() > MAX_GROUP_INVITEES {
        return Ok(bad_request(
            "Слишком много участников: максимум 199 номеров",
        ));
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
        return Ok(forbidden(
            "Только администратор группы может переименовать её",
        ));
    }

    let title = json.title.trim();
    if title.is_empty() || title.chars().count() > MAX_GROUP_TITLE_CHARS {
        return Ok(bad_request(
            "Название группы должно быть от 1 до 128 символов",
        ));
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
        return Ok(forbidden(
            "Только администратор группы может добавлять участников",
        ));
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
    // Медиа-файлы удалённого диалога тоже не должны оставаться на диске.
    // Ошибку только логируем: диалог уже удалён, ответ клиенту важнее.
    if deleted {
        if let Err(e) = media::delete_dialog(dialog_id) {
            tracing::warn!(
                "failed to delete media of empty dialog {}: {:#}",
                dialog_id,
                e
            );
        }
    }

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

/// Тело `POST /api/chats/{id}/pin`.
#[derive(Debug, serde::Deserialize)]
pub struct PinMessageBody {
    /// `null` — снять закрепление.
    #[serde(default)]
    pub message_id: Option<i64>,
}

/// Закрепляет или открепляет сообщение: `POST /api/chats/{id}/pin`.
///
/// В группе закреплять может только администратор (как и остальные
/// модерационные действия), в личном чате — любой участник.
pub async fn pin_message(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
    path: web::Path<String>,
    body: web::Json<PinMessageBody>,
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
    let is_group = matches!(db.get_dialog_type(dialog_id), Ok(Some(1)));
    if is_group {
        match db.is_dialog_admin(dialog_id, user_id) {
            Ok(true) => {}
            Ok(false) => return Ok(forbidden("only an admin can pin messages")),
            Err(e) => return Ok(server_error("failed to check admin rights", e)),
        }
    }

    let pinned_item = match body.message_id {
        Some(message_id) => {
            // Закрепить можно только живое сообщение этого же диалога.
            match db.get_message_dialog_id(message_id) {
                Ok(Some(owner)) if owner == dialog_id => {}
                Ok(_) => return Ok(not_found("message not found in this dialog")),
                Err(e) => return Ok(server_error("failed to load message", e)),
            }
            if let Err(e) = db.set_pinned_message(dialog_id, Some(message_id)) {
                return Ok(server_error("failed to pin message", e));
            }
            match db.get_message_row(message_id) {
                Ok(Some(row)) => Some(message_item_from_row(row, user_id)),
                Ok(None) => return Ok(not_found("message not found")),
                Err(e) => return Ok(server_error("failed to load message", e)),
            }
        }
        None => {
            if let Err(e) = db.set_pinned_message(dialog_id, None) {
                return Ok(server_error("failed to unpin message", e));
            }
            None
        }
    };

    let event = json!({
        "type": "dialog_pinned",
        "payload": {
            "dialog_id": dialog_id,
            "message": pinned_item,
        }
    })
    .to_string();
    state.broadcast_to_dialog(dialog_id, &event, None);

    Ok(HttpResponse::Ok().json(json!({ "ok": true, "pinned_message": pinned_item })))
}

/// Срок мьюта в unix-секундах: `0` — «навсегда».
///
/// Понимает `"1h"`, `"8h"` и `"forever"`; без значения — навсегда. Срок
/// считает сервер, чтобы клиенту не пришлось доверять своим часам.
fn parse_mute_duration(raw: Option<&str>) -> std::result::Result<i64, &'static str> {
    match raw {
        None | Some("forever") => Ok(0),
        Some("1h") => Ok(now_secs() + 3600),
        Some("8h") => Ok(now_secs() + 8 * 3600),
        Some(_) => Err("mute_duration must be 1h, 8h or forever"),
    }
}

/// Меняет личное состояние чата у текущего пользователя:
/// `PATCH /api/chats/{id}/state`.
///
/// Меняются только переданные поля. Состояние индивидуально: закрепление,
/// мьют и архив одного участника не влияют на остальных. Изменение
/// рассылается всем соединениям этого пользователя (`chat_state`), чтобы
/// другие вкладки не показывали устаревший список.
pub async fn update_chat_state(
    req: HttpRequest,
    db: web::Data<Arc<Database>>,
    state: web::Data<WebState>,
    path: web::Path<String>,
    body: web::Json<ChatStateRequest>,
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

    if let Some(pinned) = body.pinned {
        if let Err(e) = db.set_chat_pinned(user_id, dialog_id, pinned) {
            return Ok(server_error("failed to pin chat", e));
        }
    }
    if let Some(archived) = body.archived {
        if let Err(e) = db.set_chat_archived(user_id, dialog_id, archived) {
            return Ok(server_error("failed to archive chat", e));
        }
    }
    if let Some(muted) = body.muted {
        let muted_until = if muted {
            match parse_mute_duration(body.mute_duration.as_deref()) {
                Ok(until) => Some(until),
                Err(message) => return Ok(bad_request(message)),
            }
        } else {
            None
        };
        if let Err(e) = db.set_chat_muted(user_id, dialog_id, muted_until) {
            return Ok(server_error("failed to mute chat", e));
        }
    }
    if let Some(marked) = body.marked_unread {
        if let Err(e) = db.set_chat_marked_unread(user_id, dialog_id, marked) {
            return Ok(server_error("failed to mark chat unread", e));
        }
    }

    let chat = match db.get_chat_row(user_id, dialog_id) {
        Ok(Some(row)) => chat_item(row),
        Ok(None) => return Ok(not_found("dialog not found")),
        Err(e) => return Ok(server_error("failed to load chat", e)),
    };

    let event = json!({
        "type": "chat_state",
        "payload": { "chat": chat }
    })
    .to_string();
    state.broadcast_to_user(user_id, &event);

    Ok(HttpResponse::Ok().json(json!({ "ok": true, "chat": chat })))
}

/// Удаляет чат из списка пользователя: `DELETE /api/chats/{id}`.
///
/// Личный диалог мягко скрывается у пользователя (собеседник не теряет
/// историю), группу пользователь покидает. Возвращает `{"deleted": bool}` —
/// был ли диалог удалён из БД целиком (пустая группа).
pub async fn delete_chat(
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

    let deleted = match db.delete_chat(user_id, dialog_id) {
        Ok(deleted) => deleted,
        Err(e) => return Ok(server_error("failed to delete chat", e)),
    };
    if deleted {
        // Медиа-файлы удалённого диалога тоже не должны оставаться на диске.
        if let Err(e) = media::delete_dialog(dialog_id) {
            tracing::warn!(
                "failed to delete media of empty dialog {}: {:#}",
                dialog_id,
                e
            );
        }
    }

    let event = json!({
        "type": "chat_deleted",
        "payload": { "dialog_id": dialog_id, "deleted": deleted }
    })
    .to_string();
    state.broadcast_to_user(user_id, &event);

    Ok(HttpResponse::Ok().json(json!({ "ok": true, "deleted": deleted })))
}
