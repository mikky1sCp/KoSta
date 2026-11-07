// kosta-server/src/web/state.rs
//! Общие реестры WebSocket-соединений: подписки на диалоги и соединения
//! пользователей. Живут в одном экземпляре на процесс (`web::Data<WebState>`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use actix::Addr;

use crate::db::Database;
use crate::web::ws::{Disconnect, SendMessage, UserStatusUpdate, WsActor};

/// Реестр адресов, сгруппированных по числовому ключу.
///
/// Адреса в пределах ключа уникальны: одно соединение подписывается не более
/// чем на один диалог, поэтому дубликат означал бы утечку подписки.
#[derive(Debug)]
struct AddressRegistry<A> {
    map: HashMap<i64, Vec<A>>,
}

impl<A> AddressRegistry<A> {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl<A> Default for AddressRegistry<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone + PartialEq> AddressRegistry<A> {
    /// Добавляет адрес. `false` — такой адрес у ключа уже был.
    fn add(&mut self, key: i64, addr: A) -> bool {
        let list = self.map.entry(key).or_default();
        if list.contains(&addr) {
            return false;
        }
        list.push(addr);
        true
    }

    /// Удаляет адрес. `true` — если после этого ключ опустел.
    fn remove(&mut self, key: i64, addr: &A) -> bool {
        let Some(list) = self.map.get_mut(&key) else {
            return false;
        };
        list.retain(|item| item != addr);
        if list.is_empty() {
            self.map.remove(&key);
            true
        } else {
            false
        }
    }

    /// Забирает все адреса ключа (принудительное отключение).
    fn take(&mut self, key: i64) -> Vec<A> {
        self.map.remove(&key).unwrap_or_default()
    }

    fn get(&self, key: i64) -> &[A] {
        self.map.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Соединения пользователей: `user_id -> адреса`.
///
/// Нужен, чтобы переводить пользователя в offline только после закрытия
/// последнего сокета: иначе второе соединение ошибочно «гасит» первое.
#[derive(Debug)]
pub struct UserConnections<A> {
    registry: AddressRegistry<A>,
}

impl<A> UserConnections<A> {
    pub fn new() -> Self {
        Self {
            registry: AddressRegistry::new(),
        }
    }
}

impl<A> Default for UserConnections<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone + PartialEq> UserConnections<A> {
    /// Регистрирует соединение. `true` — это первое соединение пользователя.
    pub fn register(&mut self, user_id: i64, addr: A) -> bool {
        let first = self.registry.get(user_id).is_empty();
        let added = self.registry.add(user_id, addr);
        added && first
    }

    /// Снимает соединение. `true` — это было последнее соединение пользователя.
    pub fn unregister(&mut self, user_id: i64, addr: &A) -> bool {
        self.registry.remove(user_id, addr)
    }

    /// Забирает все соединения пользователя (logout / принудительный разрыв).
    pub fn take(&mut self, user_id: i64) -> Vec<A> {
        self.registry.take(user_id)
    }

    #[cfg(test)]
    fn len_for(&self, user_id: i64) -> usize {
        self.registry.get(user_id).len()
    }
}

/// Состояние web-слоя: кто на какой диалог подписан и кто авторизован.
#[derive(Clone)]
pub struct WebState {
    /// `dialog_id -> подписанные соединения`.
    connections: Arc<Mutex<AddressRegistry<Addr<WsActor>>>>,
    /// `user_id -> авторизованные соединения`.
    users: Arc<Mutex<UserConnections<Addr<WsActor>>>>,
    db: Arc<Database>,
}

impl WebState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            connections: Arc::new(Mutex::new(AddressRegistry::new())),
            users: Arc::new(Mutex::new(UserConnections::new())),
            db,
        }
    }

    pub fn add_connection(&self, dialog_id: i64, addr: Addr<WsActor>) {
        self.connections.lock().unwrap().add(dialog_id, addr);
    }

    pub fn remove_connection(&self, dialog_id: i64, addr: &Addr<WsActor>) {
        self.connections.lock().unwrap().remove(dialog_id, addr);
    }

    /// Регистрирует авторизованное соединение. `true` — первое (нужно online).
    pub fn register_user(&self, user_id: i64, addr: Addr<WsActor>) -> bool {
        self.users.lock().unwrap().register(user_id, addr)
    }

    /// Снимает соединение. `true` — у пользователя не осталось сокетов.
    pub fn unregister_user(&self, user_id: i64, addr: &Addr<WsActor>) -> bool {
        self.users.lock().unwrap().unregister(user_id, addr)
    }

    /// Рвёт все соединения пользователя (logout). Реестр чистим сразу,
    /// чтобы статус не зависел от того, успеют ли акторы обработать Disconnect.
    pub fn disconnect_user(&self, user_id: i64) {
        let addrs = self.users.lock().unwrap().take(user_id);
        if addrs.is_empty() {
            return;
        }
        for addr in &addrs {
            addr.do_send(Disconnect);
        }
        self.broadcast_status(user_id, 1);
    }

    pub fn broadcast_to_dialog(&self, dialog_id: i64, message: &str, exclude: Option<&Addr<WsActor>>) {
        let map = self.connections.lock().unwrap();
        let msg = SendMessage(message.to_string());
        for addr in map.get(dialog_id) {
            // Отправителю сообщение уже ушло через ctx.text(), иначе в UI был дубль.
            if Some(addr) == exclude {
                continue;
            }
            addr.do_send(msg.clone());
        }
    }

    // Addr — идентификатор канала с Mutex внутри, но его хеш не меняется;
    // mutable_key_type здесь ложное срабатывание.
    #[allow(clippy::mutable_key_type)]
    pub fn broadcast_status(&self, user_id: i64, status: i32) {
        // Статус — приватная информация. Рассылаем его только тем, у кого с
        // этим пользователем есть общий диалог, а не всем подключённым подряд.
        let dialogs = match self.db.get_user_dialog_ids(user_id) {
            Ok(dialogs) => dialogs,
            Err(e) => {
                tracing::error!("Failed to load dialogs for status broadcast: {}", e);
                return;
            }
        };
        let map = self.connections.lock().unwrap();
        let status_msg = UserStatusUpdate { user_id, status };
        // Один адрес может прийти из разных списков — дубли клиенту не нужны.
        let mut sent = HashSet::new();
        for dialog_id in dialogs {
            for addr in map.get(dialog_id) {
                if sent.insert(addr.clone()) {
                    addr.do_send(status_msg.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_is_deduplicated_and_removed() {
        let mut registry: AddressRegistry<u64> = AddressRegistry::new();
        assert!(registry.add(1, 10));
        assert!(!registry.add(1, 10), "повторная подписка не должна дублироваться");
        assert!(registry.add(1, 11));
        assert_eq!(registry.get(1).to_vec(), vec![10, 11]);

        assert!(!registry.remove(1, &10), "ключ ещё не опустел");
        assert!(registry.remove(1, &11), "последняя подписка очищает ключ");
        assert!(registry.get(1).is_empty());
        assert!(!registry.remove(1, &11), "повторное снятие ничего не ломает");
    }

    #[test]
    fn unregister_reports_last_user_connection() {
        let mut users: UserConnections<u64> = UserConnections::new();
        assert!(users.register(7, 100), "первое соединение — повод для online");
        assert!(!users.register(7, 100), "повторная auth не добавляет соединение");
        assert!(!users.register(7, 101), "второе соединение не считается первым");

        assert!(!users.unregister(7, &100), "есть ещё одно соединение");
        assert!(users.unregister(7, &101), "offline только после последнего");
        assert_eq!(users.len_for(7), 0);
        assert!(!users.unregister(7, &101));
    }

    #[test]
    fn take_returns_all_user_connections_once() {
        let mut users: UserConnections<u64> = UserConnections::new();
        users.register(7, 100);
        users.register(7, 101);

        let mut taken = users.take(7);
        taken.sort_unstable();
        assert_eq!(taken, vec![100, 101]);
        assert_eq!(users.len_for(7), 0);
        assert!(users.take(7).is_empty());
    }
}
