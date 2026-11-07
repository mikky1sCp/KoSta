//! `msg_container#73f1f8dc`: пакет из нескольких сообщений.
//!
//! ```text
//! msg_container#73f1f8dc messages:vector<message>
//! message#5bb8e511 msg_id:long seqno:int bytes:int body:bytes
//! ```
//!
//! Контейнеры не вкладываются друг в друга, пустой контейнер запрещён,
//! общий размер — не больше лимита транспорта (1 MiB).

use crate::error::ProtocolError;
use crate::tl::{write_i32, write_i64, write_vector_len, Reader};

/// Идентификатор `msg_container#73f1f8dc`.
pub const MSG_CONTAINER_ID: u32 = 0x73f1_f8dc;
/// Идентификатор `message#5bb8e511`.
pub const MESSAGE_ID: u32 = 0x5bb8_e511;
/// Максимум сообщений в контейнере.
pub const MAX_CONTAINER_MESSAGES: usize = 1024;
/// Максимальный размер сериализованного контейнера.
pub const MAX_CONTAINER_BYTES: usize = 1024 * 1024;

/// Сообщение внутри контейнера.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerMessage {
    /// `msg_id` вложенного сообщения.
    pub msg_id: i64,
    /// `seq_no` вложенного сообщения.
    pub seq_no: i32,
    /// Boxed TL-тело.
    pub body: Vec<u8>,
}

/// Проверяет, начинается ли `body` с идентификатора контейнера.
pub fn is_container(body: &[u8]) -> bool {
    if body.len() < 4 {
        return false;
    }
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&body[..4]);
    u32::from_le_bytes(raw) == MSG_CONTAINER_ID
}

/// Собирает `msg_container` из сообщений.
pub fn build_container(messages: &[ContainerMessage]) -> Result<Vec<u8>, ProtocolError> {
    if messages.is_empty() {
        return Err(ProtocolError::EmptyContainer);
    }
    if messages.len() > MAX_CONTAINER_MESSAGES {
        return Err(ProtocolError::TooManyMessages {
            count: messages.len(),
            max: MAX_CONTAINER_MESSAGES,
        });
    }

    let mut out = Vec::new();
    write_i32(&mut out, MSG_CONTAINER_ID as i32);
    write_vector_len(&mut out, messages.len())?;

    for message in messages {
        if message.body.is_empty() {
            return Err(ProtocolError::Malformed(
                "empty message body inside a container".into(),
            ));
        }
        if is_container(&message.body) {
            return Err(ProtocolError::NestedContainer);
        }
        let body_len = i32::try_from(message.body.len())
            .map_err(|_| ProtocolError::Malformed("message body is too long".into()))?;
        write_i32(&mut out, MESSAGE_ID as i32);
        write_i64(&mut out, message.msg_id);
        write_i32(&mut out, message.seq_no);
        write_i32(&mut out, body_len);
        out.extend_from_slice(&message.body);
    }

    if out.len() > MAX_CONTAINER_BYTES {
        return Err(ProtocolError::TooLarge {
            what: "msg_container",
            len: out.len(),
            max: MAX_CONTAINER_BYTES,
        });
    }
    Ok(out)
}

/// Разбирает `msg_container`.
pub fn parse_container(body: &[u8]) -> Result<Vec<ContainerMessage>, ProtocolError> {
    if body.len() > MAX_CONTAINER_BYTES {
        return Err(ProtocolError::TooLarge {
            what: "msg_container",
            len: body.len(),
            max: MAX_CONTAINER_BYTES,
        });
    }

    let mut reader = Reader::new(body);
    let id = reader.read_constructor_id()?;
    if id != MSG_CONTAINER_ID {
        return Err(ProtocolError::Malformed(format!(
            "expected msg_container 0x{MSG_CONTAINER_ID:08x}, got 0x{id:08x}"
        )));
    }

    let count = reader.read_vector_len()?;
    if count == 0 {
        return Err(ProtocolError::EmptyContainer);
    }
    if count > MAX_CONTAINER_MESSAGES {
        return Err(ProtocolError::TooManyMessages {
            count,
            max: MAX_CONTAINER_MESSAGES,
        });
    }

    let mut messages = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let message_id = reader.read_constructor_id()?;
        if message_id != MESSAGE_ID {
            return Err(ProtocolError::Malformed(format!(
                "expected message 0x{MESSAGE_ID:08x}, got 0x{message_id:08x}"
            )));
        }
        let msg_id = reader.read_i64()?;
        let seq_no = reader.read_i32()?;
        let body_len = reader.read_i32()?;
        if body_len <= 0 {
            return Err(ProtocolError::Malformed(format!(
                "invalid message body length inside a container: {body_len}"
            )));
        }
        let message_body = reader.take(body_len as usize)?.to_vec();
        if is_container(&message_body) {
            return Err(ProtocolError::NestedContainer);
        }
        messages.push(ContainerMessage {
            msg_id,
            seq_no,
            body: message_body,
        });
    }

    if !reader.is_empty() {
        return Err(ProtocolError::Malformed(format!(
            "{} trailing bytes after msg_container",
            reader.remaining()
        )));
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(msg_id: i64, seq_no: i32, body: &[u8]) -> ContainerMessage {
        ContainerMessage {
            msg_id,
            seq_no,
            body: body.to_vec(),
        }
    }

    #[test]
    fn container_roundtrip() {
        let messages = vec![
            message(10, 1, &[1, 0, 0, 0, 0xAA]),
            message(14, 3, &[2, 0, 0, 0]),
        ];
        let encoded = build_container(&messages).unwrap();
        let parsed = parse_container(&encoded).unwrap();
        assert_eq!(parsed, messages);
    }

    #[test]
    fn empty_container_is_rejected() {
        assert!(matches!(
            build_container(&[]),
            Err(ProtocolError::EmptyContainer)
        ));
    }

    #[test]
    fn too_many_messages_are_rejected() {
        let messages: Vec<_> = (0..=MAX_CONTAINER_MESSAGES)
            .map(|i| message(i as i64, 1, &[0, 0, 0, 0]))
            .collect();
        assert!(matches!(
            build_container(&messages),
            Err(ProtocolError::TooManyMessages { .. })
        ));
    }

    #[test]
    fn nested_containers_are_rejected() {
        let inner = build_container(&[message(1, 1, &[0, 0, 0, 0])]).unwrap();
        let outer = [message(2, 3, &inner)];
        assert!(matches!(
            build_container(&outer),
            Err(ProtocolError::NestedContainer)
        ));
    }

    #[test]
    fn empty_message_body_is_rejected() {
        let messages = [message(1, 1, &[])];
        assert!(build_container(&messages).is_err());
    }

    #[test]
    fn malformed_containers_are_rejected() {
        let encoded = build_container(&[message(1, 1, &[0, 0, 0, 0])]).unwrap();

        let mut wrong_id = encoded.clone();
        wrong_id[0] ^= 0xFF;
        assert!(parse_container(&wrong_id).is_err());

        assert!(parse_container(&encoded[..6]).is_err());

        let mut trailing = encoded;
        trailing.push(0);
        assert!(parse_container(&trailing).is_err());
    }

    #[test]
    fn container_detection_works() {
        let encoded = build_container(&[message(1, 1, &[0, 0, 0, 0])]).unwrap();
        assert!(is_container(&encoded));
        assert!(!is_container(&[0, 0, 0, 0]));
        assert!(!is_container(&[1, 2]));
    }
}
