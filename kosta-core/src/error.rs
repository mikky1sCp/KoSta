use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    UnexpectedEof,
    InvalidConstructor(u32),
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO error: {}", e),
            Error::UnexpectedEof => write!(f, "Unexpected end of input"),
            Error::InvalidConstructor(id) => write!(f, "Unknown constructor: 0x{:08x}", id),
            Error::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}