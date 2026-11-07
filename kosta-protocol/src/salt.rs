//! Серверный salt и `new_session_created`.
//!
//! В MTProto 2.0 `salt` и `session_id` входят в расшифрованный plaintext
//! каждого сообщения. Клиент начинает с `salt = 0`, получает настоящий salt
//! в `new_session_created` и дальше подписывает им сообщения. Сервер принимает
//! текущий salt и (на время перехода) предыдущий.

use rand::Rng;

use crate::error::ProtocolError;
use crate::tl::{write_i32, write_i64, Reader};

/// Идентификатор `new_session_created#9ec20908`.
pub const NEW_SESSION_CREATED_ID: u32 = 0x9ec2_0908;

/// Состояние серверного salt.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServerSalt {
    current: i64,
    previous: Option<i64>,
}

impl ServerSalt {
    /// Новое состояние с начальным salt (обычно 0).
    pub fn new(initial: i64) -> Self {
        ServerSalt {
            current: initial,
            previous: None,
        }
    }

    /// Текущий salt.
    pub fn current(&self) -> i64 {
        self.current
    }

    /// Предыдущий salt, если он ещё принимается.
    pub fn previous(&self) -> Option<i64> {
        self.previous
    }

    /// Устанавливает новый salt; старый ещё принимается до следующего update.
    pub fn update(&mut self, new_salt: i64) {
        self.previous = Some(self.current);
        self.current = new_salt;
    }

    /// Принимается ли `salt` от клиента.
    pub fn is_acceptable(&self, salt: i64) -> bool {
        salt == self.current || self.previous == Some(salt)
    }
}

/// Случайный серверный salt (никогда не 0).
pub fn generate_server_salt() -> i64 {
    let mut rng = rand::thread_rng();
    loop {
        let salt: i64 = rng.gen();
        if salt != 0 {
            return salt;
        }
    }
}

/// Случайный `session_id` клиента (никогда не 0).
pub fn generate_session_id() -> i64 {
    generate_server_salt()
}

/// Содержимое `new_session_created`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewSessionCreated {
    /// `msg_id` первого сообщения клиента в новой сессии.
    pub first_msg_id: i64,
    /// Уникальный идентификатор сессии (для обнаружения пересоздания).
    pub unique_id: i64,
    /// Salt, которым клиент должен подписывать сообщения.
    pub server_salt: i64,
}

/// Собирает TL-объект `new_session_created`.
pub fn build_new_session_created(first_msg_id: i64, unique_id: i64, server_salt: i64) -> Vec<u8> {
    let mut out = Vec::new();
    write_i32(&mut out, NEW_SESSION_CREATED_ID as i32);
    write_i64(&mut out, first_msg_id);
    write_i64(&mut out, unique_id);
    write_i64(&mut out, server_salt);
    out
}

/// Разбирает TL-объект `new_session_created`.
pub fn parse_new_session_created(body: &[u8]) -> Result<NewSessionCreated, ProtocolError> {
    let mut reader = Reader::new(body);
    let id = reader.read_constructor_id()?;
    if id != NEW_SESSION_CREATED_ID {
        return Err(ProtocolError::Malformed(format!(
            "expected new_session_created 0x{NEW_SESSION_CREATED_ID:08x}, got 0x{id:08x}"
        )));
    }
    Ok(NewSessionCreated {
        first_msg_id: reader.read_i64()?,
        unique_id: reader.read_i64()?,
        server_salt: reader.read_i64()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_lifecycle_accepts_current_and_previous() {
        let mut salt = ServerSalt::new(0);
        assert_eq!(salt.current(), 0);
        assert!(salt.is_acceptable(0));
        assert!(!salt.is_acceptable(42));

        salt.update(42);
        assert_eq!(salt.current(), 42);
        assert_eq!(salt.previous(), Some(0));
        assert!(salt.is_acceptable(42));
        assert!(salt.is_acceptable(0));
        assert!(!salt.is_acceptable(43));

        salt.update(77);
        assert!(salt.is_acceptable(77));
        assert!(salt.is_acceptable(42));
        assert!(!salt.is_acceptable(0));
    }

    #[test]
    fn generated_salts_are_non_zero() {
        for _ in 0..100 {
            assert_ne!(generate_server_salt(), 0);
            assert_ne!(generate_session_id(), 0);
        }
    }

    #[test]
    fn new_session_created_roundtrip() {
        let body = build_new_session_created(101, 202, 303);
        let parsed = parse_new_session_created(&body).unwrap();
        assert_eq!(
            parsed,
            NewSessionCreated {
                first_msg_id: 101,
                unique_id: 202,
                server_salt: 303,
            }
        );
    }

    #[test]
    fn new_session_created_rejects_wrong_object() {
        let mut body = build_new_session_created(1, 2, 3);
        body[0] ^= 0xFF;
        assert!(parse_new_session_created(&body).is_err());
    }

    #[test]
    fn new_session_created_truncated_is_rejected() {
        let body = build_new_session_created(1, 2, 3);
        assert!(parse_new_session_created(&body[..10]).is_err());
    }
}
