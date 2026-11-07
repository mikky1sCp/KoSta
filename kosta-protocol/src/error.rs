//! Ошибки слоя MTProto 2.0 (кадры, контейнеры, RPC, msg_id/seq_no).

use std::fmt;

/// Ошибка разбора или сборки MTProto-сообщения.
#[derive(Debug)]
pub enum ProtocolError {
    /// Ошибка крипто-слоя (неверный `msg_key`, длина шифротекста и т.п.).
    Crypto(kosta_crypto::CryptoError),
    /// Структурно некорректные данные.
    Malformed(String),
    /// Данных меньше, чем требуется для поля.
    Truncated { needed: usize, available: usize },
    /// Поле/объект превышает допустимый размер.
    TooLarge {
        what: &'static str,
        len: usize,
        max: usize,
    },
    /// В контейнере слишком много сообщений.
    TooManyMessages { count: usize, max: usize },
    /// Пустой контейнер запрещён.
    EmptyContainer,
    /// Контейнер внутри контейнера запрещён.
    NestedContainer,
    /// Некорректный `msg_id` (чётность, монотонность).
    MsgId(String),
    /// Некорректный `seq_no`.
    SeqNo(String),
    /// `msg_id` за пределами окна времени.
    TimeSkew { msg_id_secs: i64, server_secs: i64 },
    /// Повторное сообщение (уже обработанный `msg_id`).
    Duplicate,
    /// Прикладная RPC-ошибка, пришедшая в `rpc_result`.
    Rpc { code: i32, message: String },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::Crypto(e) => write!(f, "crypto error: {e}"),
            ProtocolError::Malformed(message) => write!(f, "malformed message: {message}"),
            ProtocolError::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated message: need {needed} bytes, have {available}"
                )
            }
            ProtocolError::TooLarge { what, len, max } => {
                write!(f, "{what} is too large: {len} bytes (max {max})")
            }
            ProtocolError::TooManyMessages { count, max } => {
                write!(f, "too many messages in container: {count} (max {max})")
            }
            ProtocolError::EmptyContainer => write!(f, "empty message container"),
            ProtocolError::NestedContainer => write!(f, "nested message containers are forbidden"),
            ProtocolError::MsgId(message) => write!(f, "bad msg_id: {message}"),
            ProtocolError::SeqNo(message) => write!(f, "bad seq_no: {message}"),
            ProtocolError::TimeSkew {
                msg_id_secs,
                server_secs,
            } => write!(
                f,
                "msg_id timestamp {msg_id_secs} is out of the allowed window (now {server_secs})"
            ),
            ProtocolError::Duplicate => write!(f, "duplicate message"),
            ProtocolError::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<kosta_crypto::CryptoError> for ProtocolError {
    fn from(e: kosta_crypto::CryptoError) -> Self {
        ProtocolError::Crypto(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_does_not_panic() {
        let errors = [
            ProtocolError::Malformed("x".into()),
            ProtocolError::Truncated {
                needed: 10,
                available: 2,
            },
            ProtocolError::TooLarge {
                what: "frame",
                len: 10,
                max: 5,
            },
            ProtocolError::TooManyMessages { count: 2, max: 1 },
            ProtocolError::EmptyContainer,
            ProtocolError::NestedContainer,
            ProtocolError::MsgId("x".into()),
            ProtocolError::SeqNo("x".into()),
            ProtocolError::TimeSkew {
                msg_id_secs: 1,
                server_secs: 2,
            },
            ProtocolError::Duplicate,
            ProtocolError::Rpc {
                code: 400,
                message: "bad".into(),
            },
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
