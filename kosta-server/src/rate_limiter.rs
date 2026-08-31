// kosta-server/src/rate_limiter.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use lazy_static::lazy_static;

pub struct RateLimiter<K: Eq + std::hash::Hash + Clone> {
    window: Duration,
    max_requests: usize,
    storage: Arc<Mutex<HashMap<K, Vec<Instant>>>>,
}

impl<K: Eq + std::hash::Hash + Clone> RateLimiter<K> {
    pub fn new(window: Duration, max_requests: usize) -> Self {
        Self {
            window,
            max_requests,
            storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, key: K) -> Result<(), &'static str> {
        let now = Instant::now();
        let mut storage = self.storage.lock().unwrap();
        let timestamps = storage.entry(key).or_insert_with(Vec::new);
        // Удаляем записи старше окна
        timestamps.retain(|&t| now.duration_since(t) < self.window);
        if timestamps.len() >= self.max_requests {
            Err("Rate limit exceeded")
        } else {
            timestamps.push(now);
            Ok(())
        }
    }
}

// Глобальные ограничители
lazy_static! {
    pub static ref WS_MESSAGE_LIMITER: RateLimiter<i64> =
        RateLimiter::new(Duration::from_secs(1), 10);   // 10 сообщений в секунду на пользователя
    pub static ref HTTP_LIMITER: RateLimiter<String> =
        RateLimiter::new(Duration::from_secs(60), 100); // 100 запросов в минуту с IP
}

/// Проверка лимита. Возвращает `Ok(())` если разрешено, иначе `Err` с сообщением.
pub fn check_limit<K: Eq + std::hash::Hash + Clone>(
    limiter: &RateLimiter<K>,
    key: K,
) -> Result<(), &'static str> {
    limiter.check(key)
}