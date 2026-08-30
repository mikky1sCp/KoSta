use std::fmt;
use cipher::InvalidLength;

#[derive(Debug)]
pub enum CryptoError {
    Core(kosta_core::Error),
    Aes(InvalidLength),
    PaddingError,
    InvalidAuthKey,
    DhError(String),
    Custom(String),
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for CryptoError {}

impl From<kosta_core::Error> for CryptoError {
    fn from(e: kosta_core::Error) -> Self {
        CryptoError::Core(e)
    }
}

impl From<InvalidLength> for CryptoError {
    fn from(e: InvalidLength) -> Self {
        CryptoError::Aes(e)
    }
}