// kosta-server/src/session_cache.rs
use anyhow::Result;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use crate::db::Database;
use crate::session_store::SessionData;
use kosta_core::tl::types::{Int128, Int256};
use kosta_crypto::AuthKey;
use kosta_crypto::session_crypto::SessionCrypto;

pub struct SessionCache {
    cache: Mutex<LruCache<i64, SessionData>>,
    db: Arc<Database>,
}

impl SessionCache {
    pub fn new(capacity: usize, db: Arc<Database>) -> Self {
        let cache = LruCache::new(
            NonZeroUsize::new(capacity)
                .expect("Session cache capacity must be non-zero")
        );
        Self {
            cache: Mutex::new(cache),
            db,
        }
    }

    pub fn get_session(&self, auth_key_id: i64) -> Result<Option<SessionData>> {
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(session) = cache.get(&auth_key_id) {
                return Ok(Some(session.clone()));
            }
        }
        if let Some(full) = self.db.get_full_session(auth_key_id)? {
            let nonce = Int128(full.nonce);
            let server_nonce = Int128(full.server_nonce);
            let new_nonce = Int256(full.new_nonce);

            let crypto = SessionCrypto::from_keys(
                full.client_write_key,
                full.client_mac_key,
                full.server_write_key,
                full.server_mac_key,
                full.send_counter,
                full.recv_counter,
            );

            let auth_key = AuthKey(full.auth_key);

            let recent = Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(1000).expect("cache size must be non-zero")
            )));

            let session = SessionData {
                auth_key,
                auth_key_id,
                user_id: full.user_id,
                server_salt: full.server_salt,
                nonce,
                server_nonce,
                new_nonce,
                crypto,
                recv_seq_no: full.recv_seq_no,
                last_recv_msg_id: full.last_msg_id,
                send_counter: full.send_counter,
                recv_counter: full.recv_counter,
                send_seq_no: full.send_seq_no,
                msg_id_counter: full.msg_id_counter,
                recent_msg_ids: recent,
            };
            let mut cache = self.cache.lock().unwrap();
            cache.put(auth_key_id, session.clone());
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    pub fn update_session(&self, auth_key_id: i64, session: SessionData) -> Result<()> {
        self.db.save_session_full(
            auth_key_id,
            session.user_id,
            session.server_salt,
            &session.nonce.0,
            &session.server_nonce.0,
            &session.new_nonce.0,
            session.recv_seq_no,
            &session.auth_key.0,
            &session.crypto.client_write_key,
            &session.crypto.client_mac_key,
            &session.crypto.server_write_key,
            &session.crypto.server_mac_key,
            session.last_recv_msg_id,
            session.send_counter,
            session.recv_counter,
            session.send_seq_no,
            session.msg_id_counter,
        )?;
        let mut cache = self.cache.lock().unwrap();
        cache.put(auth_key_id, session);
        Ok(())
    }
}