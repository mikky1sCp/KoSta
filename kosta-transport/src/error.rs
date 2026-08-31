// kosta-transport/src/error.rs
use std::fmt;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Crypto(kosta_crypto::CryptoError),
    Core(kosta_core::Error),
    Tls(native_tls::Error),          // <-- добавить
    TlsHandshake(std::io::Error),    // <-- добавить (для HandshakeError мы преобразуем в Io)
    InvalidLength,
    UnexpectedEof,
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

impl From<kosta_crypto::CryptoError> for TransportError {
    fn from(e: kosta_crypto::CryptoError) -> Self {
        TransportError::Crypto(e)
    }
}

impl From<kosta_core::Error> for TransportError {
    fn from(e: kosta_core::Error) -> Self {
        TransportError::Core(e)
    }
}

// Добавляем преобразование для native_tls::Error
impl From<native_tls::Error> for TransportError {
    fn from(e: native_tls::Error) -> Self {
        TransportError::Tls(e)
    }
}

// Для HandshakeError<TcpStream> мы можем преобразовать в Io, так как он содержит io::Error внутри
impl From<native_tls::HandshakeError<std::net::TcpStream>> for TransportError {
    fn from(e: native_tls::HandshakeError<std::net::TcpStream>) -> Self {
        // Можно попытаться извлечь io::Error, если это возможно, иначе обернуть в Custom
        if let native_tls::HandshakeError::Failure(e) = e {
            TransportError::Tls(e)
        } else {
            TransportError::Custom("TLS handshake interrupted".into())
        }
    }
}