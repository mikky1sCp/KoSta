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

#[derive(Debug, Serialize)]
pub struct ChatItem {
    pub id: i64,
    pub title: String,
    pub is_group: bool,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
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

#[derive(Debug, Deserialize)]
pub struct GetHistoryQuery {
    pub chat_id: i64,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
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
}
