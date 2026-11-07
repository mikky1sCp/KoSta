// kosta-server/src/rate_limiter.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use lazy_static::lazy_static;

/// При превышении числа ключей словарь чистится от просроченных записей,
/// иначе он растёт бесконечно (например, ключ — IP клиента).
const MAX_TRACKED_KEYS: usize = 4096;

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

        if storage.len() > MAX_TRACKED_KEYS {
            storage.retain(|_, timestamps| {
                timestamps.retain(|t| now.duration_since(*t) < self.window);
                !timestamps.is_empty()
            });
        }

        let timestamps = storage.entry(key).or_default();
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
    /// Рукопожатие на TCP-протоколе: до авторизации соединение анонимно,
    /// а ReqDHParams стоит серверу 2048-битного modpow.
    pub static ref TCP_HANDSHAKE_LIMITER: RateLimiter<String> =
        RateLimiter::new(Duration::from_secs(60), 30);  // 30 рукопожатий в минуту с IP
}

/// Проверка лимита. Возвращает `Ok(())` если разрешено, иначе `Err` с сообщением.
pub fn check_limit<K: Eq + std::hash::Hash + Clone>(
    limiter: &RateLimiter<K>,
    key: K,
) -> Result<(), &'static str> {
    limiter.check(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_after_limit() {
        let limiter = RateLimiter::new(Duration::from_secs(60), 2);
        assert!(limiter.check("ip".to_string()).is_ok());
        assert!(limiter.check("ip".to_string()).is_ok());
        assert!(limiter.check("ip".to_string()).is_err());
        // Другой ключ не должен страдать от чужого лимита
        assert!(limiter.check("other".to_string()).is_ok());
    }

    #[test]
    fn limiter_forgets_old_requests() {
        let limiter = RateLimiter::new(Duration::from_millis(50), 1);
        assert!(limiter.check("ip".to_string()).is_ok());
        assert!(limiter.check("ip".to_string()).is_err());

        std::thread::sleep(Duration::from_millis(80));
        assert!(
            limiter.check("ip".to_string()).is_ok(),
            "после окна лимит должен снова разрешать"
        );
    }
}