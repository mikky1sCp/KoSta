use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use actix::Addr;
use crate::web::ws::{WsActor, SendMessage, UserStatusUpdate};

#[derive(Clone)]
pub struct WebState {
    pub connections: Arc<Mutex<HashMap<i64, Vec<Addr<WsActor>>>>>, // dialog_id -> список адресов
}

impl WebState {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn add_connection(&self, dialog_id: i64, addr: Addr<WsActor>) {
        let mut map = self.connections.lock().unwrap();
        map.entry(dialog_id).or_insert_with(Vec::new).push(addr);
    }

    pub fn broadcast_to_dialog(&self, dialog_id: i64, message: &str, _exclude_user: Option<i64>) {
        let map = self.connections.lock().unwrap();
        if let Some(vec) = map.get(&dialog_id) {
            let msg = SendMessage(message.to_string());
            for addr in vec {
                let _ = addr.do_send(msg.clone());
            }
        }
    }

    pub fn broadcast_status(&self, user_id: i64, status: i32) {
        let map = self.connections.lock().unwrap();
        let status_msg = UserStatusUpdate { user_id, status };
        for (_, vec) in map.iter() {
            for addr in vec {
                let _ = addr.do_send(status_msg.clone());
            }
        }
    }
}