// kosta-server/src/rate_limiter.rs
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

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
        // Отравленный mutex (паника в соседнем потоке, пока лок был взят) не
        // должен ронять лимитер: данные — просто список отметок времени,
        // поэтому продолжаем работу с внутренним состоянием.
        let mut storage = self
            .storage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

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

// Глобальные ограничители. `LazyLock` из std — тот же ленивый одноразовый
// init, что давал lazy_static, но без внешней зависимости.
pub static WS_MESSAGE_LIMITER: LazyLock<RateLimiter<i64>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(1), 10)); // 10 сообщений в секунду на пользователя
/// Индикатор набора: клиент шлёт его не чаще ~2 с, но сервер ограничивает
/// поток отдельно, иначе «печатает…» стал бы каналом спама для подписчиков.
pub static WS_TYPING_LIMITER: LazyLock<RateLimiter<i64>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(1), 10)); // 10 typing-событий в секунду на пользователя
/// Попытки авторизации в WebSocket (токен может быть перебираемым/устаревшим).
pub static WS_AUTH_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(60), 10)); // 10 попыток auth в минуту с IP
/// Общий лимит кадров WebSocket с одного IP: до auth соединение анонимно,
/// поэтому даже мусорный JSON не должен бесконечно писать в лог.
pub static WS_FRAME_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(1), 60)); // 60 кадров в секунду с IP
pub static HTTP_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(60), 100)); // 100 запросов в минуту с IP
/// Логин/регистрация: bcrypt на каждый запрос, поэтому лимит отдельный и жёсткий.
pub static LOGIN_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(60), 10)); // 10 попыток входа в минуту с IP
/// Перебор пароля конкретного аккаунта: лимит по IP не спасает от
/// распределённого перебора (боты с разных адресов бьют по одному номеру),
/// поэтому считаем ещё и по нормализованному номеру.
pub static LOGIN_ACCOUNT_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(15 * 60), 10)); // 10 попыток на номер за 15 минут
/// Загрузка медиа: запись на диск и сообщение в историю. Считаем по
/// пользователю (а не по IP), чтобы общий HTTP-лимит не был единственной
/// защитой от заваливания диска файлами.
pub static MEDIA_LIMITER: LazyLock<RateLimiter<i64>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(60), 30)); // 30 загрузок в минуту на пользователя
/// Регистрация по бинарному протоколу (SignUp): дороже, чем кажется (bcrypt).
pub static SIGNUP_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(3600), 5)); // 5 регистраций в час с IP
/// Рукопожатие на TCP-протоколе: до авторизации соединение анонимно,
/// а ReqDHParams стоит серверу 2048-битного modpow.
pub static TCP_HANDSHAKE_LIMITER: LazyLock<RateLimiter<String>> =
    LazyLock::new(|| RateLimiter::new(Duration::from_secs(60), 30)); // 30 рукопожатий в минуту с IP

/// Проверка лимита. Возвращает `Ok(())` если разрешено, иначе `Err` с сообщением.
///
/// `KOSTA_RATE_LIMIT_DISABLED=1` полностью отключает лимиты. Это нужно только
/// нагрузочным тестам (много соединений с одного IP иначе упирается в лимит
/// авторизации), и переменная не должна выставляться в продакшене.
pub fn check_limit<K: Eq + std::hash::Hash + Clone>(
    limiter: &RateLimiter<K>,
    key: K,
) -> Result<(), &'static str> {
    if limits_disabled() {
        return Ok(());
    }
    limiter.check(key)
}

/// Отключены ли лимиты переменной окружения (читается один раз).
pub fn limits_disabled() -> bool {
    *LIMITS_DISABLED
}

static LIMITS_DISABLED: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("KOSTA_RATE_LIMIT_DISABLED")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
});

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[test]
    fn storage_does_not_grow_unboundedly() {
        // Много разных ключей с малым числом запросов в каждом: словарь должен
        // уметь вычищаться после истечения окна, а не блокировать «свежие» ключи.
        let limiter = RateLimiter::new(Duration::from_millis(5), 1000);
        for i in 0..(MAX_TRACKED_KEYS + 512) {
            assert!(limiter.check(format!("key-{i}")).is_ok());
        }
        // Окно истекает — следующий вызов запускает retain и чистит всё.
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            limiter.check("fresh".to_string()).is_ok(),
            "после чистки просроченных ключей новый ключ должен проходить"
        );
    }

    #[test]
    fn limiter_is_correct_under_concurrency() {
        let limiter = Arc::new(RateLimiter::new(Duration::from_secs(10), 2));
        let accepted = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let limiter = Arc::clone(&limiter);
            let accepted = Arc::clone(&accepted);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    if limiter.check("same-key".to_string()).is_ok() {
                        accepted.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        // При макс. 2 запросах в окне суммарно должно пройти ровно 2, несмотря
        // на гонку 8 потоков.
        assert_eq!(
            accepted.load(Ordering::Relaxed),
            2,
            "ограничитель должен быть корректным при конкурентном доступе"
        );
    }

    #[test]
    fn limiter_survives_poisoned_mutex() {
        // Паника в потоке, держащем лок, отравляет mutex. Лимитер хранит лишь
        // отметки времени, так что после такой паники он должен продолжать
        // работать, а не паниковать на каждом вызове.
        let limiter = Arc::new(RateLimiter::new(Duration::from_secs(60), 2));
        let poisoned = Arc::clone(&limiter);
        let join_result = std::thread::spawn(move || {
            let _guard = poisoned.storage.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();
        assert!(join_result.is_err(), "поток должен был упасть");

        assert!(limiter.check("ip".to_string()).is_ok());
        assert!(limiter.check("ip".to_string()).is_ok());
        assert!(limiter.check("ip".to_string()).is_err());
    }

    #[test]
    fn login_account_limiter_blocks_after_account_limit() {
        // Глобальный ограничитель: ключ уникален для прогона, чтобы тесты не
        // влияли друг на друга. Обращаемся к check() напрямую — check_limit()
        // зависит от KOSTA_RATE_LIMIT_DISABLED и в тестах недетерминирован.
        let key = format!("test-account-{}", rand::random::<u64>());
        for attempt in 0..10 {
            assert!(
                LOGIN_ACCOUNT_LIMITER.check(key.clone()).is_ok(),
                "попытка {} должна проходить",
                attempt + 1
            );
        }
        assert!(
            LOGIN_ACCOUNT_LIMITER.check(key).is_err(),
            "11-я попытка должна блокироваться"
        );
    }
}
