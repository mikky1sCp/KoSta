use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub phone: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub auth_key_id: Option<i64>,
    pub user_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsMessage {
    pub action: String,
    pub payload: serde_json::Value,
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
pub struct SendMessagePayload {
    pub chat_id: i64,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct GetHistoryQuery {
    pub chat_id: i64,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
