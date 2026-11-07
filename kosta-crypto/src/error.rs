use std::fmt;

use cipher::InvalidLength;

#[derive(Debug)]
pub enum CryptoError {
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

impl From<InvalidLength> for CryptoError {
    fn from(e: InvalidLength) -> Self {
        CryptoError::Aes(e)
    }
}