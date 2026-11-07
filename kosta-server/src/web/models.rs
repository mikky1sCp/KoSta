// kosta-server/src/web/models.rs
use serde::{Deserialize, Serialize};

/// Тело POST /api/login.
///
/// `register` по умолчанию false: зная только телефон, нельзя создать аккаунт.
/// Неизвестные поля serde игнорирует, поэтому старые клиенты не ломаются.
#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub phone: String,
    pub password: String,
    #[serde(default)]
    pub register: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub token: Option<String>,
    pub auth_key_id: Option<i64>,
    pub user_id: Option<i64>,
    pub error: Option<String>,
}

/// Элемент списка чатов (`GET /api/chats`) и карточка внутри `ChatDetails`.
///
/// `other_user_id` и `is_online` относятся только к личным диалогам: у группы
/// нет единственного «собеседника», поэтому для неё эти поля не сериализуются
/// вовсе (а не приходят как null/false).
#[derive(Debug, Serialize)]
pub struct ChatItem {
    pub id: i64,
    pub title: String,
    pub is_group: bool,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
    /// Число непрочитанных входящих сообщений именно этого пользователя.
    pub unread_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_user_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_online: Option<bool>,
}

/// Участник диалога в ответе `GET /api/chats/{id}`.
#[derive(Debug, Serialize)]
pub struct ParticipantItem {
    pub user_id: i64,
    pub phone: String,
    pub is_admin: bool,
    pub is_online: bool,
}

/// Детали диалога: карточка чата, участники и роль текущего пользователя.
#[derive(Debug, Serialize)]
pub struct ChatDetails {
    pub chat: ChatItem,
    pub participants: Vec<ParticipantItem>,
    pub is_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct MessageItem {
    pub id: i64,
    pub sender_id: i64,
    pub text: String,
    pub timestamp: i64,
    pub is_outgoing: bool,
    pub read: bool,
    pub delivered: bool,
    pub media_path: Option<String>,
    pub media_type: Option<String>,
    pub media_size: Option<i64>,
    pub is_media: bool,
}

/// Ответ `GET /api/me`.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: i64,
    pub phone: String,
    pub is_online: bool,
}

/// Ответ `GET /api/users/lookup`.
#[derive(Debug, Serialize)]
pub struct LookupResponse {
    pub user_id: i64,
    pub phone: String,
}

#[derive(Debug, Deserialize)]
pub struct GetHistoryQuery {
    pub chat_id: i64,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Тело `POST /api/chats/private`.
///
/// Поля с `default` нужны не для «пустого запроса», а чтобы отсутствие поля
/// давало наш JSON-400 из валидации, а не текстовую ошибку экстрактора actix.
#[derive(Debug, Deserialize)]
pub struct CreatePrivateChatRequest {
    #[serde(default)]
    pub phone: String,
}

/// Тело `POST /api/chats/group`. Пустой `phones` — валидный случай:
/// группа из одного создателя.
#[derive(Debug, Deserialize)]
pub struct CreateGroupChatRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub phones: Vec<String>,
}

/// Тело `PATCH /api/chats/{id}`.
#[derive(Debug, Deserialize)]
pub struct RenameChatRequest {
    #[serde(default)]
    pub title: String,
}

/// Тело `POST /api/chats/{id}/participants`.
#[derive(Debug, Deserialize)]
pub struct AddParticipantRequest {
    #[serde(default)]
    pub phone: String,
}

/// Query `GET /api/users/lookup`. `Option`, чтобы отсутствие параметра
/// обрабатывалось нашим JSON-400.
#[derive(Debug, Deserialize)]
pub struct LookupQuery {
    pub phone: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_register_defaults_to_false() {
        let req: LoginRequest =
            serde_json::from_str(r#"{"phone":"+79990000000","password":"testpass123"}"#).unwrap();
        assert!(!req.register, "без явного register аккаунт создаваться не должен");
    }

    #[test]
    fn login_request_accepts_register_and_ignores_unknown_fields() {
        let req: LoginRequest = serde_json::from_str(
            r#"{"phone":"+79990000000","password":"testpass123","register":true,"extra":"ignored"}"#,
        )
        .unwrap();
        assert!(req.register);
        assert_eq!(req.phone, "+79990000000");
    }

    #[test]
    fn login_request_requires_password() {
        assert!(serde_json::from_str::<LoginRequest>(r#"{"phone":"+79990000000"}"#).is_err());
    }

    fn chat_item(is_group: bool, other_user_id: Option<i64>, is_online: Option<bool>) -> ChatItem {
        ChatItem {
            id: 12,
            title: "+79990000001".to_string(),
            is_group,
            last_message: Some("привет".to_string()),
            last_timestamp: Some(1_794_057_537),
            unread_count: 2,
            other_user_id,
            is_online,
        }
    }

    #[test]
    fn chat_item_serializes_unread_and_private_fields() {
        let value = serde_json::to_value(chat_item(false, Some(8), Some(true))).unwrap();
        assert_eq!(value["id"], 12);
        assert_eq!(value["unread_count"], 2);
        assert_eq!(value["other_user_id"], 8);
        assert_eq!(value["is_online"], true);
    }

    #[test]
    fn chat_item_keeps_offline_false_for_private_chat() {
        // false — значимое значение: собеседник существует, но офлайн.
        // Поэтому поле сериализуется как Some(false), а не пропадает.
        let value = serde_json::to_value(chat_item(false, Some(8), Some(false))).unwrap();
        assert_eq!(value["is_online"], false);
    }

    #[test]
    fn chat_item_omits_private_fields_for_group() {
        let value = serde_json::to_value(chat_item(true, None, None)).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("other_user_id"), "у группы нет собеседника");
        assert!(!object.contains_key("is_online"), "у группы нет статуса собеседника");
        assert_eq!(value["unread_count"], 2);
    }

    #[test]
    fn chat_details_serialize_participants_and_role() {
        let details = ChatDetails {
            chat: chat_item(true, None, None),
            participants: vec![
                ParticipantItem {
                    user_id: 7,
                    phone: "+79990000000".to_string(),
                    is_admin: true,
                    is_online: true,
                },
                ParticipantItem {
                    user_id: 8,
                    phone: "+79990000001".to_string(),
                    is_admin: false,
                    is_online: false,
                },
            ],
            is_admin: true,
        };
        let value = serde_json::to_value(details).unwrap();
        assert_eq!(value["is_admin"], true);
        assert_eq!(value["participants"][0]["is_admin"], true);
        assert_eq!(value["participants"][1]["phone"], "+79990000001");
        assert_eq!(value["chat"]["is_group"], true);
    }

    #[test]
    fn group_request_defaults_are_empty() {
        let req: CreateGroupChatRequest =
            serde_json::from_str(r#"{"title":"Друзья"}"#).unwrap();
        assert_eq!(req.title, "Друзья");
        assert!(req.phones.is_empty(), "группа без приглашённых валидна");
    }

    #[test]
    fn missing_title_is_empty_string_not_extractor_error() {
        let req: RenameChatRequest = serde_json::from_str("{}").unwrap();
        assert!(req.title.is_empty(), "пустое название отсеет валидация хендлера");
    }

    #[test]
    fn me_and_lookup_responses_are_snake_case() {
        let me = serde_json::to_value(MeResponse {
            user_id: 7,
            phone: "+79990000000".to_string(),
            is_online: false,
        })
        .unwrap();
        assert_eq!(me["user_id"], 7);
        assert_eq!(me["is_online"], false);

        let lookup = serde_json::to_value(LookupResponse {
            user_id: 8,
            phone: "+79990000001".to_string(),
        })
        .unwrap();
        assert_eq!(lookup["user_id"], 8);
        assert_eq!(lookup["phone"], "+79990000001");
    }
}