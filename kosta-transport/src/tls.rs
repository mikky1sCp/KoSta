// kosta-transport/src/tls.rs
//! TLS-транспорт.
//!
//! Проверка сертификата и имени хоста включена **по умолчанию**.
//! Флаг `allow_invalid_certs` нужен только для локальной отладки с
//! самоподписанным сертификатом: с ним TLS перестаёт защищать от MITM.

use std::net::TcpStream;

use native_tls::{TlsAcceptor, TlsConnector, TlsStream};

use crate::error::TransportError;
use crate::StreamTransport;

pub type TlsTransport = StreamTransport<TlsStream<TcpStream>>;

/// Подключение к серверу по TLS.
///
/// * `domain` — имя, против которого проверяется сертификат (SNI).
/// * `allow_invalid_certs` — **только для dev**, по умолчанию `false`.
pub fn connect_tls(
    host: &str,
    port: u16,
    domain: &str,
    allow_invalid_certs: bool,
) -> Result<TlsTransport, TransportError> {
    let stream = TcpStream::connect((host, port))?;

    let mut builder = TlsConnector::builder();
    if allow_invalid_certs {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    let connector = builder.build()?;
    let tls_stream = connector.connect(domain, stream)?;
    Ok(StreamTransport::new(tls_stream))
}

/// Обёртка принятого TCP-соединения в серверный TLS.
pub fn accept_tls(stream: TcpStream, acceptor: &TlsAcceptor) -> Result<TlsTransport, TransportError> {
    let tls_stream = acceptor.accept(stream)?;
    Ok(StreamTransport::new(tls_stream))
}
