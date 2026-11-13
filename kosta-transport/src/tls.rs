// kosta-transport/src/tls.rs
//! TLS-подключение клиента (проверка сертификата и имени хоста включена).

use std::net::TcpStream;
use std::time::Duration;

use native_tls::{TlsConnector, TlsStream};

use crate::error::TransportError;

/// «Голый» TLS-поток — нужен транспортам, которые кадрируют сами
/// (например, MTProto-abridged).
pub type ClientTlsStream = TlsStream<TcpStream>;

/// Устанавливает TLS-соединение и возвращает поток без обёртки-транспорта.
///
/// * `domain` — имя, против которого проверяется сертификат (SNI).
/// * `allow_invalid_certs` — **только для dev**, по умолчанию `false`.
/// * `timeout` — таймауты чтения/записи на нижележащем TCP-сокете.
pub fn connect_tls_stream(
    host: &str,
    port: u16,
    domain: &str,
    allow_invalid_certs: bool,
    timeout: Option<Duration>,
) -> Result<ClientTlsStream, TransportError> {
    let stream = TcpStream::connect((host, port))?;
    if let Some(timeout) = timeout {
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
    }

    let mut builder = TlsConnector::builder();
    if allow_invalid_certs {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    let connector = builder.build()?;
    Ok(connector.connect(domain, stream)?)
}
