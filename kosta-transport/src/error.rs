// kosta-transport/src/error.rs
use std::fmt;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Tls(native_tls::Error),
    Custom(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        TransportError::Io(e)
    }
}

impl From<native_tls::Error> for TransportError {
    fn from(e: native_tls::Error) -> Self {
        TransportError::Tls(e)
    }
}

impl From<native_tls::HandshakeError<std::net::TcpStream>> for TransportError {
    fn from(e: native_tls::HandshakeError<std::net::TcpStream>) -> Self {
        if let native_tls::HandshakeError::Failure(e) = e {
            TransportError::Tls(e)
        } else {
            TransportError::Custom("TLS handshake interrupted".into())
        }
    }
}