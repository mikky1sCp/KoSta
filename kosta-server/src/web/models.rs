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
    /// Отображаемое имя, если пользователь его задал.
    pub display_name: Option<String>,
    pub is_admin: bool,
    pub is_online: bool,
}

/// Детали диалога: карточка чата, участники и роль текущего пользователя.
#[derive(Debug, Serialize)]
pub struct ChatDetails {
    pub chat: ChatItem,
    pub participants: Vec<ParticipantItem>,
    pub is_admin: bool,
    /// Закреплённое сообщение диалога (если есть).
    pub pinned_message: Option<MessageItem>,
}

/// Элемент истории и ответа на загрузку медиа.
///
/// Ключ хранения (`media_path` в БД) наружу не отдаётся: скачивание — только
/// по id сообщения через `GET /api/media/{message_id}`. Поэтому в API есть
/// `media_name` (безопасное исходное имя), но нет пути на сервере.
#[derive(Debug, Serialize)]
pub struct MessageItem {
    pub id: i64,
    pub sender_id: i64,
    pub text: String,
    pub timestamp: i64,
    pub is_outgoing: bool,
    pub read: bool,
    pub delivered: bool,
    pub media_name: Option<String>,
    pub media_type: Option<String>,
    pub media_size: Option<i64>,
    pub is_media: bool,
    /// Сообщение, на которое это отвечает (цитата), если есть.
    pub reply_to_id: Option<i64>,
    /// Текст цитируемого сообщения; `None`, если оно удалено.
    pub reply_to_text: Option<String>,
    /// Автор цитируемого сообщения.
    pub reply_to_sender_id: Option<i64>,
    /// Было ли цитируемое сообщение медиа (тогда текст может быть подписью).
    pub reply_to_is_media: bool,
    /// Время последнего редактирования, unix seconds.
    pub edited_at: Option<i64>,
    /// Реакции, сгруппированные по эмодзи.
    pub reactions: Vec<ReactionItem>,
}

/// Реакция на сообщение: эмодзи, число поставивших и признак «моя».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReactionItem {
    pub emoji: String,
    pub count: i64,
    pub me: bool,
}

/// Ответ `GET /api/me`.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: i64,
    pub phone: String,
    /// Отображаемое имя (None — показываем телефон).
    pub display_name: Option<String>,
    /// Публичный `@username` (None — не задан).
    pub username: Option<String>,
    /// Загружен ли аватар (картинка — `GET /api/avatars/{user_id}`).
    pub has_avatar: bool,
    pub is_online: bool,
}

/// Человек в результатах `GET /api/users?q=`.
///
/// Телефон намеренно не отдаётся: по username можно найти собеседника,
/// но не узнать его номер.
#[derive(Debug, Serialize)]
pub struct UserSearchItem {
    pub user_id: i64,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub has_avatar: bool,
    pub is_online: bool,
}

/// Сообщение из глобального поиска (`GET /api/search?q=`) с привязкой к чату.
#[derive(Debug, Serialize)]
pub struct GlobalSearchHitItem {
    pub chat_id: i64,
    pub chat_title: String,
    pub message: MessageItem,
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
/// Достаточно `phone` или `username`: по username можно начать чат с
/// человеком, найденным через поиск людей. Поля с `default` нужны не для
/// «пустого запроса», а чтобы отсутствие поля давало наш JSON-400 из
/// валидации, а не текстовую ошибку экстрактора actix.
#[derive(Debug, Deserialize)]
pub struct CreatePrivateChatRequest {
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub username: String,
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

/// Query `POST /api/chats/{id}/media?name=...&caption=...`.
///
/// `name` обязателен (проверяет хендлер), `caption` — необязательная подпись.
/// Оба `Option`, чтобы отсутствие параметра давало наш JSON-400, а не
/// текстовую ошибку экстрактора actix.
#[derive(Debug, Deserialize)]
pub struct UploadMediaQuery {
    pub name: Option<String>,
    pub caption: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_register_defaults_to_false() {
        let req: LoginRequest =
            serde_json::from_str(r#"{"phone":"+79990000000","password":"testpass123"}"#).unwrap();
        assert!(
            !req.register,
            "без явного register аккаунт создаваться не должен"
        );
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
        assert!(
            !object.contains_key("other_user_id"),
            "у группы нет собеседника"
        );
        assert!(
            !object.contains_key("is_online"),
            "у группы нет статуса собеседника"
        );
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
                    display_name: Some("Аня".to_string()),
                    is_admin: true,
                    is_online: true,
                },
                ParticipantItem {
                    user_id: 8,
                    phone: "+79990000001".to_string(),
                    display_name: None,
                    is_admin: false,
                    is_online: false,
                },
            ],
            is_admin: true,
            pinned_message: None,
        };
        let value = serde_json::to_value(details).unwrap();
        assert_eq!(value["is_admin"], true);
        assert_eq!(value["participants"][0]["is_admin"], true);
        assert_eq!(value["participants"][1]["phone"], "+79990000001");
        assert_eq!(value["chat"]["is_group"], true);
    }

    #[test]
    fn group_request_defaults_are_empty() {
        let req: CreateGroupChatRequest = serde_json::from_str(r#"{"title":"Друзья"}"#).unwrap();
        assert_eq!(req.title, "Друзья");
        assert!(req.phones.is_empty(), "группа без приглашённых валидна");
    }

    #[test]
    fn missing_title_is_empty_string_not_extractor_error() {
        let req: RenameChatRequest = serde_json::from_str("{}").unwrap();
        assert!(
            req.title.is_empty(),
            "пустое название отсеет валидация хендлера"
        );
    }

    #[test]
    fn me_and_lookup_responses_are_snake_case() {
        let me = serde_json::to_value(MeResponse {
            user_id: 7,
            phone: "+79990000000".to_string(),
            display_name: Some("Аня".to_string()),
            username: Some("anya".to_string()),
            has_avatar: true,
            is_online: false,
        })
        .unwrap();
        assert_eq!(me["user_id"], 7);
        assert_eq!(me["is_online"], false);
        assert_eq!(me["display_name"], "Аня");
        assert_eq!(me["username"], "anya");
        assert_eq!(me["has_avatar"], true);

        let lookup = serde_json::to_value(LookupResponse {
            user_id: 8,
            phone: "+79990000001".to_string(),
        })
        .unwrap();
        assert_eq!(lookup["user_id"], 8);
        assert_eq!(lookup["phone"], "+79990000001");
    }

    #[test]
    fn media_message_item_matches_contract_and_hides_storage_path() {
        let item = MessageItem {
            id: 105,
            sender_id: 8,
            text: String::new(),
            timestamp: 1_794_057_537,
            is_outgoing: false,
            read: false,
            delivered: false,
            media_name: Some("фото.png".to_string()),
            media_type: Some("image/png".to_string()),
            media_size: Some(1024),
            is_media: true,
            reply_to_id: None,
            reply_to_text: None,
            reply_to_sender_id: None,
            reply_to_is_media: false,
            edited_at: None,
            reactions: Vec::new(),
        };
        let value = serde_json::to_value(item).unwrap();
        assert_eq!(value["media_name"], "фото.png");
        assert_eq!(value["media_type"], "image/png");
        assert_eq!(value["media_size"], 1024);
        assert_eq!(value["is_media"], true);
        assert_eq!(value["text"], "");
        assert!(
            value.get("media_path").is_none(),
            "путь хранения не должен утекать в API"
        );
    }

    #[test]
    fn plain_message_item_keeps_null_media_fields() {
        let item = MessageItem {
            id: 1,
            sender_id: 7,
            text: "hello".to_string(),
            timestamp: 10,
            is_outgoing: true,
            read: true,
            delivered: false,
            media_name: None,
            media_type: None,
            media_size: None,
            is_media: false,
            reply_to_id: None,
            reply_to_text: None,
            reply_to_sender_id: None,
            reply_to_is_media: false,
            edited_at: None,
            reactions: Vec::new(),
        };
        let value = serde_json::to_value(item).unwrap();
        assert!(value["media_name"].is_null());
        assert!(value["media_type"].is_null());
        assert!(value["media_size"].is_null());
        assert_eq!(value["is_media"], false);
        assert_eq!(value["text"], "hello");
    }

    #[test]
    fn upload_media_query_parses_name_and_caption() {
        let query: UploadMediaQuery =
            serde_json::from_str(r#"{"name":"фото.png","caption":"подпись"}"#).unwrap();
        assert_eq!(query.name.as_deref(), Some("фото.png"));
        assert_eq!(query.caption.as_deref(), Some("подпись"));

        let empty: UploadMediaQuery = serde_json::from_str("{}").unwrap();
        assert!(empty.name.is_none());
        assert!(empty.caption.is_none());
    }
}
