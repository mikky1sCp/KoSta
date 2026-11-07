// kosta-server/src/session_cache.rs
use anyhow::Result;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use kosta_core::tl::types::{Int128, Int256};
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_crypto::AuthKey;

use crate::db::{Database, FullSession, SessionRow};
use crate::session_store::SessionData;

pub struct SessionCache {
    cache: Mutex<LruCache<i64, SessionData>>,
    db: Arc<Database>,
}

impl SessionCache {
    pub fn new(capacity: usize, db: Arc<Database>) -> Self {
        let cache = LruCache::new(
            NonZeroUsize::new(capacity).expect("Session cache capacity must be non-zero"),
        );
        Self {
            cache: Mutex::new(cache),
            db,
        }
    }

    fn session_from_full(full: FullSession) -> SessionData {
        let crypto = SessionCrypto::from_keys(
            full.client_write_key,
            full.client_mac_key,
            full.server_write_key,
            full.server_mac_key,
            full.send_counter,
            full.recv_counter,
        );

        SessionData {
            auth_key: AuthKey(full.auth_key),
            user_id: full.user_id,
            server_salt: full.server_salt,
            nonce: Int128(full.nonce),
            server_nonce: Int128(full.server_nonce),
            new_nonce: Int256(full.new_nonce),
            crypto,
            recv_seq_no: full.recv_seq_no,
            last_recv_msg_id: full.last_msg_id,
            send_counter: full.send_counter,
            recv_counter: full.recv_counter,
            send_seq_no: full.send_seq_no,
            msg_id_counter: full.msg_id_counter,
            recent_msg_ids: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(1000).expect("cache size must be non-zero"),
            ))),
        }
    }

    pub fn get_session(&self, auth_key_id: i64) -> Result<Option<SessionData>> {
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(session) = cache.get(&auth_key_id) {
                return Ok(Some(session.clone()));
            }
        }
        match self.db.get_full_session(auth_key_id)? {
            Some(full) => {
                let session = Self::session_from_full(full);
                let mut cache = self.cache.lock().unwrap();
                cache.put(auth_key_id, session.clone());
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    /// Атомарная read-modify-write операция над сессией.
    ///
    /// Раньше обработчик делал `get_session` (клон) → мутация → `update_session`,
    /// и два соединения с одним auth_key затирали счётчики друг друга.
    /// Здесь блокировка держится на всё время операции, состояние кэша при
    /// ошибке откатывается, а в БД уходит одна запись вместо трёх.
    ///
    /// Возвращает `None`, если сессии с таким auth_key нет.
    pub fn mutate_session<T, F>(&self, auth_key_id: i64, f: F) -> Result<Option<T>>
    where
        F: FnOnce(&mut SessionData) -> Result<T>,
    {
        let mut cache = self.cache.lock().unwrap();

        let mut session = match cache.get(&auth_key_id) {
            Some(session) => session.clone(),
            None => match self.db.get_full_session(auth_key_id)? {
                Some(full) => Self::session_from_full(full),
                None => return Ok(None),
            },
        };
        let backup = session.clone();

        match f(&mut session) {
            Ok(value) => {
                self.persist(auth_key_id, &session)?;
                cache.put(auth_key_id, session);
                Ok(Some(value))
            }
            Err(e) => {
                // Откат: неудачная попытка не должна оставлять следов в кэше.
                cache.put(auth_key_id, backup);
                Err(e)
            }
        }
    }

    fn persist(&self, auth_key_id: i64, session: &SessionData) -> Result<()> {
        self.db.save_session_full(
            auth_key_id,
            &SessionRow {
                user_id: session.user_id,
                server_salt: session.server_salt,
                nonce: &session.nonce.0,
                server_nonce: &session.server_nonce.0,
                new_nonce: &session.new_nonce.0,
                recv_seq_no: session.recv_seq_no,
                auth_key: &session.auth_key.0,
                client_write_key: &session.crypto.client_write_key,
                client_mac_key: &session.crypto.client_mac_key,
                server_write_key: &session.crypto.server_write_key,
                server_mac_key: &session.crypto.server_mac_key,
                last_msg_id: session.last_recv_msg_id,
                send_counter: session.send_counter,
                recv_counter: session.recv_counter,
                send_seq_no: session.send_seq_no,
                msg_id_counter: session.msg_id_counter,
            },
        )
    }

    pub fn update_session(&self, auth_key_id: i64, session: SessionData) -> Result<()> {
        self.persist(auth_key_id, &session)?;
        let mut cache = self.cache.lock().unwrap();
        cache.put(auth_key_id, session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    const AUTH_KEY_ID: i64 = 4242;

    struct TestEnv {
        cache: Arc<SessionCache>,
        user_id: i64,
        path: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{}", self.path.display(), suffix));
            }
        }
    }

    fn test_env() -> TestEnv {
        let path =
            std::env::temp_dir().join(format!("kosta_cache_test_{}.db", rand::random::<u64>()));
        let secrets = Arc::new(crate::secrets::SecretsBox::from_key([0x24u8; 32]));
        let db = Arc::new(
            Database::new(path.to_str().expect("temp path is utf-8"), secrets).expect("test db"),
        );
        let user_id = db.create_user("+cache-test", "x").expect("test user");
        TestEnv {
            cache: Arc::new(SessionCache::new(16, db)),
            user_id,
            path,
        }
    }

    fn sample_session(user_id: i64) -> SessionData {
        SessionData {
            auth_key: AuthKey([7u8; 256]),
            user_id,
            server_salt: 0,
            nonce: Int128([1; 16]),
            server_nonce: Int128([2; 16]),
            new_nonce: Int256([3; 32]),
            crypto: SessionCrypto::from_keys([1; 32], [2; 32], [3; 32], [4; 32], 0, 0),
            recv_seq_no: -1,
            last_recv_msg_id: 0,
            send_counter: 0,
            recv_counter: 0,
            send_seq_no: -1,
            msg_id_counter: 0,
            recent_msg_ids: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(100).expect("non-zero"),
            ))),
        }
    }

    #[test]
    fn concurrent_mutations_do_not_lose_updates() {
        let env = test_env();
        env.cache
            .update_session(AUTH_KEY_ID, sample_session(env.user_id))
            .unwrap();

        let threads = 8;
        let per_thread = 50;
        let mut handles = Vec::new();
        for _ in 0..threads {
            let cache = env.cache.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..per_thread {
                    cache
                        .mutate_session(AUTH_KEY_ID, |session| {
                            session.msg_id_counter += 1;
                            Ok(())
                        })
                        .expect("mutate")
                        .expect("session exists");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("thread");
        }

        let expected = threads * per_thread;
        let in_cache = env.cache.get_session(AUTH_KEY_ID).unwrap().unwrap();
        assert_eq!(
            in_cache.msg_id_counter, expected,
            "потерянные обновления в кэше — гонка на клонах вернулась"
        );

        let in_db = env.cache.db.get_full_session(AUTH_KEY_ID).unwrap().unwrap();
        assert_eq!(
            in_db.msg_id_counter, expected,
            "БД разошлась с кэшем после конкурентных обновлений"
        );
    }

    #[test]
    fn failed_mutation_rolls_back() {
        let env = test_env();
        env.cache
            .update_session(AUTH_KEY_ID, sample_session(env.user_id))
            .unwrap();

        let result = env.cache.mutate_session(AUTH_KEY_ID, |session| {
            session.msg_id_counter = 999;
            session.last_recv_msg_id = 123;
            Err::<(), _>(anyhow::anyhow!("отклонённое сообщение"))
        });
        assert!(result.is_err());

        let in_cache = env.cache.get_session(AUTH_KEY_ID).unwrap().unwrap();
        assert_eq!(in_cache.msg_id_counter, 0, "откат должен вернуть счётчик");
        assert_eq!(in_cache.last_recv_msg_id, 0, "откат должен вернуть msg_id");
    }
}
