//! Клиентский крейт KoSta.
//!
//! [`MtprotoSession`] — клиент MTProto 2.0: хендшейк, RPC, acks, retries, TLS.
//! Это единственный бинарный протокол после cutover (этап 6 миграции).

pub mod error;
pub mod mtproto;
pub mod server_keys;

pub use error::KostaError;
pub use mtproto::{ConnectOptions, MtprotoSession};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_is_verified_by_default() {
        // Регресс-защита: проверка сертификата не должна отключаться по умолчанию.
        let opts = ConnectOptions::default();
        assert!(
            !opts.allow_invalid_certs,
            "проверка TLS-сертификата обязана быть включена по умолчанию"
        );
        assert!(!opts.use_tls, "TLS не включается без явного запроса");
    }
}
