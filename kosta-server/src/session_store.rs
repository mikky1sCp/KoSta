// kosta-server/src/session_store.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use kosta_crypto::AuthKey;
use kosta_core::tl::types::{Int128, Int256};
use kosta_crypto::session_crypto::SessionCrypto;

#[derive(Clone)]
pub struct SessionData {
    pub auth_key: AuthKey,
    pub auth_key_id: i64,
    pub user_id: i64,
    pub server_salt: i64,
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub new_nonce: Int256,
    pub crypto: SessionCrypto,
    pub recv_seq_no: i32,
    pub last_recv_msg_id: i64,
    pub send_counter: u32,
    pub recv_counter: u32,
    pub send_seq_no: i32,        // добавлено
    pub msg_id_counter: u32,     // добавлено
}

pub type SessionStore = Arc<Mutex<HashMap<i64, SessionData>>>;

pub fn new_session_store() -> SessionStore {
    Arc::new(Mutex::new(HashMap::new()))
}