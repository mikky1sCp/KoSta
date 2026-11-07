// kosta-transport/src/lib.rs
//! Транспорты KoSta: MTProto-abridged кадрирование и TLS-подключение.
//! Легаси-«abridged» (длина в байтах, без init-байта) удалён на этапе 6.

pub mod error;
pub mod mtproto_abridged;
pub mod tls;

pub use error::TransportError;
