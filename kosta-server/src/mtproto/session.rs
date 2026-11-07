//! Состояние сессии MTProto 2.0 и хранилище сессий.
//!
//! В памяти держим горячие сессии, в SQLite — сырой `auth_key` (зашифрованный
//! мастер-ключом), salt и счётчики. Счётчики пишутся не на каждый кадр, а
//! периодически и при закрытии соединения — иначе SQLite становится узким
//! местом при контейнерах.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

use kosta_protocol::ack::{AckTracker, ResponseCache};
use kosta_protocol::msg_id::{MsgIdGenerator, MsgIdValidator, SeqNoGenerator};
use kosta_protocol::salt::{generate_server_salt, ServerSalt};

use crate::db::{Database, MtprotoSessionRow};

/// Сколько обработанных кадров между записями счётчиков в БД.
pub const PERSIST_EVERY_MESSAGES: u32 = 32;

/// Горячая сессия MTProto 2.0.
pub struct MtprotoSession {
    /// `auth_key_id` — первичный ключ сессии.
    pub auth_key_id: i64,
    /// Сырой 2048-битный DH-секрет.
    pub auth_key: [u8; 256],
    /// Пользователь (`0` — сессия ещё не зарегистрирована).
    pub user_id: i64,
    /// Серверный salt, который уходит клиенту в `new_session_created`.
    pub server_salt: i64,
    /// Какие salt'ы принимаем от клиента (0 до первого `new_session_created`).
    pub salt_state: ServerSalt,
    /// Отправляли ли уже `new_session_created` в этом соединении.
    pub salt_sent: bool,
    /// `unique_id` сессии для `new_session_created`.
    pub unique_id: i64,
    /// Валидатор входящих `msg_id` (клиентская чётность).
    pub validator: MsgIdValidator,
    /// Генератор серверных `msg_id` (чётность 1).
    pub msg_ids: MsgIdGenerator,
    /// Генератор `seq_no`.
    pub seq: SeqNoGenerator,
    /// Накопленные `msgs_ack`.
    pub acks: AckTracker,
    /// Кэш ответов на повторные `msg_id`.
    pub responses: ResponseCache,
    /// Последний `seq_no`, полученный от клиента.
    pub last_client_seq: i32,
    /// Время последней активности.
    pub last_used: Instant,
    /// Сколько кадров обработано с последней записи в БД.
    pub processed_since_persist: u32,
}

impl MtprotoSession {
    /// Создаёт сессию из строки БД (в том числе после рестарта сервера).
    pub fn from_row(row: MtprotoSessionRow) -> Self {
        let validator = MsgIdValidator::with_last(0, row.last_msg_id);
        let msg_ids = MsgIdGenerator::with_state(1, row.msg_id_counter);
        let content_count = row.send_seq_no.max(0) as u32;
        let mut salt_state = ServerSalt::new(0);
        salt_state.update(row.salt);
        MtprotoSession {
            auth_key_id: row.auth_key_id,
            auth_key: row.auth_key,
            user_id: row.user_id,
            server_salt: row.salt,
            salt_state,
            salt_sent: row.last_msg_id > 0,
            unique_id: generate_server_salt(),
            validator,
            msg_ids,
            seq: SeqNoGenerator::with_state(content_count, 0),
            acks: AckTracker::new(),
            responses: ResponseCache::new(1000),
            last_client_seq: row.recv_seq_no,
            last_used: Instant::now(),
            processed_since_persist: 0,
        }
    }

    /// Сессия сразу после успешного DH-хендшейка.
    pub fn new_session(auth_key_id: i64, auth_key: [u8; 256], salt: i64) -> Self {
        MtprotoSession {
            auth_key_id,
            auth_key,
            user_id: 0,
            server_salt: salt,
            salt_state: ServerSalt::new(0),
            salt_sent: false,
            unique_id: generate_server_salt(),
            validator: MsgIdValidator::with_last(0, 0),
            msg_ids: MsgIdGenerator::server(),
            seq: SeqNoGenerator::new(),
            acks: AckTracker::new(),
            responses: ResponseCache::new(1000),
            last_client_seq: -1,
            last_used: Instant::now(),
            processed_since_persist: 0,
        }
    }

    /// Снимок для записи в БД.
    pub fn to_row(&self) -> MtprotoSessionRow {
        MtprotoSessionRow {
            auth_key_id: self.auth_key_id,
            user_id: self.user_id,
            salt: self.server_salt,
            auth_key: self.auth_key,
            last_msg_id: self.validator.last().unwrap_or(0),
            recv_seq_no: self.last_client_seq,
            send_seq_no: self.seq.state().0 as i32,
            msg_id_counter: self.msg_ids.counter(),
        }
    }

    /// Пишет счётчики сессии в БД.
    pub fn persist(&self, db: &Database) -> Result<()> {
        let row = self.to_row();
        db.save_mtproto_session_state(
            row.auth_key_id,
            row.last_msg_id,
            row.recv_seq_no,
            row.send_seq_no,
            row.msg_id_counter,
        )
    }

    /// Отмечает активность и, если накопилось достаточно кадров, просит
    /// вызывающий код сохранить счётчики.
    pub fn touch(&mut self) -> bool {
        self.last_used = Instant::now();
        self.processed_since_persist += 1;
        if self.processed_since_persist >= PERSIST_EVERY_MESSAGES {
            self.processed_since_persist = 0;
            true
        } else {
            false
        }
    }
}

/// Хранилище сессий: LRU-кэш в памяти + SQLite.
pub struct SessionStore {
    cache: Mutex<HashMap<i64, Arc<Mutex<MtprotoSession>>>>,
    db: Arc<Database>,
}

impl SessionStore {
    /// Создаёт хранилище поверх базы.
    pub fn new(db: Arc<Database>) -> Self {
        SessionStore {
            cache: Mutex::new(HashMap::new()),
            db,
        }
    }

    /// Возвращает сессию из памяти или загружает из БД.
    pub fn get(&self, auth_key_id: i64) -> Result<Option<Arc<Mutex<MtprotoSession>>>> {
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(session) = cache.get(&auth_key_id) {
                return Ok(Some(Arc::clone(session)));
            }
        }

        let Some(row) = self.db.load_mtproto_session(auth_key_id)? else {
            return Ok(None);
        };
        let session = Arc::new(Mutex::new(MtprotoSession::from_row(row)));
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        Ok(Some(
            cache
                .entry(auth_key_id)
                .or_insert_with(|| Arc::clone(&session))
                .clone(),
        ))
    }

    /// Создаёт новую сессию после успешного хендшейка.
    pub fn create(
        &self,
        auth_key_id: i64,
        auth_key: [u8; 256],
        salt: i64,
    ) -> Result<Arc<Mutex<MtprotoSession>>> {
        let session = MtprotoSession::new_session(auth_key_id, auth_key, salt);
        self.db.save_mtproto_session(&session.to_row())?;
        let shared = Arc::new(Mutex::new(session));
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(auth_key_id, Arc::clone(&shared));
        Ok(shared)
    }
}
