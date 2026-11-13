// kosta-server/src/web/client_addr.rs
//! IP клиента для лимитов и логов.
//!
//! При прямом TCP-соединении адрес берётся у пира. За reverse-proxy (nginx,
//! Caddy, любой балансировщик) все клиенты видны с одного адреса, и лимиты
//! «на IP» превращаются в общий лимит на всех — достаточно одного шумного
//! клиента, чтобы получить 429 всем.
//!
//! Поэтому адрес клиента может браться из `Forwarded` / `X-Forwarded-For`,
//! но только когда сервер явно настроен доверять прокси (`web.trust_proxy`
//! в конфиге). Без этого флага заголовок подделывается кем угодно: клиент
//! прислал бы новый IP на каждый запрос и обошёл лимиты.

use actix_web::HttpRequest;
use std::sync::atomic::{AtomicBool, Ordering};

/// Доверяем ли заголовкам прокси. Инициализируется один раз при старте
/// web-сервера (`web.trust_proxy` в конфиге).
static TRUST_PROXY: AtomicBool = AtomicBool::new(false);

/// Настраивает доверие к заголовкам прокси. Вызывается при старте сервера.
pub fn set_trust_proxy(trust: bool) {
    TRUST_PROXY.store(trust, Ordering::Relaxed);
}

/// Доверяет ли сервер заголовкам прокси (для тестов и диагностики).
pub fn trust_proxy() -> bool {
    TRUST_PROXY.load(Ordering::Relaxed)
}

/// IP клиента: адрес из `Forwarded`/`X-Forwarded-For` при включённом
/// `web.trust_proxy`, иначе — адрес TCP-пира. Строка `"unknown"` —
/// запасной вариант, когда адрес недоступен (например, в тестах).
pub fn client_ip(req: &HttpRequest) -> String {
    let info = req.connection_info();
    let addr = if trust_proxy() {
        info.realip_remote_addr()
    } else {
        info.peer_addr()
    };
    addr.unwrap_or("unknown").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    /// Флаг — глобальный, поэтому тесты модуля не должны выполняться
    /// параллельно и видеть чужое значение.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn request_with_forwarded_header(ip: &str) -> HttpRequest {
        TestRequest::default()
            .peer_addr("127.0.0.1:5555".parse().expect("valid socket addr"))
            .insert_header(("x-forwarded-for", ip))
            .to_http_request()
    }

    #[test]
    fn direct_connection_uses_peer_address() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_trust_proxy(false);
        let req = request_with_forwarded_header("203.0.113.7");
        assert_eq!(client_ip(&req), "127.0.0.1");
    }

    #[test]
    fn trusted_proxy_uses_forwarded_header() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_trust_proxy(true);
        let req = request_with_forwarded_header("203.0.113.7");
        assert_eq!(client_ip(&req), "203.0.113.7");
        set_trust_proxy(false);
    }

    #[test]
    fn missing_address_falls_back_to_unknown() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_trust_proxy(false);
        let req = TestRequest::default().to_http_request();
        assert_eq!(client_ip(&req), "unknown");
    }
}
