// =============================================================================
// kosta/src/error.rs
// =============================================================================
use std::fmt;

#[derive(Debug)]
pub enum KostaError {
    Io(std::io::Error),
    Transport(kosta_transport::TransportError),
    Crypto(kosta_crypto::CryptoError),
    Core(kosta_core::Error),
    Protocol(String),
    ConnectionFailed(String),
}

impl fmt::Display for KostaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for KostaError {}

impl From<std::io::Error> for KostaError {
    fn from(e: std::io::Error) -> Self {
        KostaError::Io(e)
    }
}

impl From<kosta_transport::TransportError> for KostaError {
    fn from(e: kosta_transport::TransportError) -> Self {
        KostaError::Transport(e)
    }
}

impl From<kosta_crypto::CryptoError> for KostaError {
    fn from(e: kosta_crypto::CryptoError) -> Self {
        KostaError::Crypto(e)
    }
}

impl From<kosta_core::Error> for KostaError {
    fn from(e: kosta_core::Error) -> Self {
        KostaError::Core(e)
    }
}