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
        map.entry(dialog_id).or_default().push(addr);
    }

    pub fn broadcast_to_dialog(&self, dialog_id: i64, message: &str, exclude: Option<&Addr<WsActor>>) {
        let map = self.connections.lock().unwrap();
        if let Some(vec) = map.get(&dialog_id) {
            let msg = SendMessage(message.to_string());
            for addr in vec {
                // Отправителю сообщение уже ушло через ctx.text(), иначе в UI был дубль.
                if Some(addr) == exclude {
                    continue;
                }
                addr.do_send(msg.clone());
            }
        }
    }

    pub fn broadcast_status(&self, user_id: i64, status: i32) {
        let map = self.connections.lock().unwrap();
        let status_msg = UserStatusUpdate { user_id, status };
        for vec in map.values() {
            for addr in vec {
                addr.do_send(status_msg.clone());
            }
        }
    }
}