//! Кадры MTProto 2.0: незашифрованный (хендшейк) и зашифрованный.
//!
//! Незашифрованный кадр:
//! ```text
//! auth_key_id:long = 0
//! msg_id:long
//! msg_len:int
//! body:bytes
//! ```
//!
//! Зашифрованный кадр:
//! ```text
//! auth_key_id:long
//! msg_key:int128
//! encrypted_data:bytes   // AES-256-IGE
//! ```
//!
//! Plaintext зашифрованного кадра:
//! ```text
//! salt:long
//! session_id:long
//! msg_id:long
//! seq_no:int
//! body:TL
//! padding
//! ```

use kosta_crypto::mtproto2::{self, AuthKey, Direction};

use crate::error::ProtocolError;
use crate::tl::{write_i32, write_i64, Reader};

/// Длина `auth_key_id`.
pub const AUTH_KEY_ID_LEN: usize = 8;
/// Длина `msg_key`.
pub const MSG_KEY_LEN: usize = mtproto2::MSG_KEY_LEN;
/// Длина заголовка зашифрованного кадра: `auth_key_id + msg_key`.
pub const HEADER_LEN: usize = AUTH_KEY_ID_LEN + MSG_KEY_LEN;
/// Длина plaintext-заголовка: `salt + session_id + msg_id + seq_no`.
pub const PLAINTEXT_HEADER_LEN: usize = 8 + 8 + 8 + 4;
/// Максимальный размер кадра (совпадает с лимитом abridged-транспорта).
pub const MAX_FRAME_LEN: usize = 1024 * 1024;
/// Минимальный размер шифротекста: один блок AES.
pub const MIN_CIPHERTEXT_LEN: usize = 16;

/// Незашифрованный кадр (фаза хендшейка).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainFrame {
    /// `msg_id` сообщения.
    pub msg_id: i64,
    /// Boxed TL-тело.
    pub body: Vec<u8>,
}

/// Собирает незашифрованный кадр.
pub fn build_plain_frame(msg_id: i64, body: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let total = AUTH_KEY_ID_LEN + 8 + 4 + body.len();
    if total > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge {
            what: "plain frame",
            len: total,
            max: MAX_FRAME_LEN,
        });
    }
    let mut out = Vec::with_capacity(total);
    write_i64(&mut out, 0);
    write_i64(&mut out, msg_id);
    write_i32(&mut out, body.len() as i32);
    out.extend_from_slice(body);
    Ok(out)
}

/// Разбирает незашифрованный кадр.
pub fn parse_plain_frame(frame: &[u8]) -> Result<PlainFrame, ProtocolError> {
    if frame.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge {
            what: "plain frame",
            len: frame.len(),
            max: MAX_FRAME_LEN,
        });
    }
    let mut reader = Reader::new(frame);
    let auth_key_id = reader.read_i64()?;
    if auth_key_id != 0 {
        return Err(ProtocolError::Malformed(format!(
            "unencrypted frame must have auth_key_id = 0, got {auth_key_id}"
        )));
    }
    let msg_id = reader.read_i64()?;
    let len = reader.read_i32()?;
    if len < 0 {
        return Err(ProtocolError::Malformed(format!(
            "negative plain frame body length: {len}"
        )));
    }
    if len as usize != reader.remaining() {
        return Err(ProtocolError::Malformed(format!(
            "plain frame body length {len} does not match remaining {}",
            reader.remaining()
        )));
    }
    Ok(PlainFrame {
        msg_id,
        body: reader.take(len as usize)?.to_vec(),
    })
}

/// Заголовок и полезная нагрузка расшифрованного кадра.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedFrame {
    /// `auth_key_id` из кадра.
    pub auth_key_id: i64,
    /// Серверный salt.
    pub salt: i64,
    /// Идентификатор сессии.
    pub session_id: i64,
    /// `msg_id` сообщения.
    pub msg_id: i64,
    /// `seq_no` сообщения.
    pub seq_no: i32,
    /// TL-тело вместе с паддингом (длину тела определяет разбор TL).
    pub payload: Vec<u8>,
}

/// Шифрует и собирает кадр.
#[allow(clippy::too_many_arguments)]
pub fn seal_frame(
    auth_key: &AuthKey,
    direction: Direction,
    auth_key_id: i64,
    session_id: i64,
    salt: i64,
    msg_id: i64,
    seq_no: i32,
    body: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let plaintext_len = PLAINTEXT_HEADER_LEN + body.len() + mtproto2::MAX_PADDING;
    if HEADER_LEN + plaintext_len > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge {
            what: "encrypted frame",
            len: HEADER_LEN + plaintext_len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut plaintext = Vec::with_capacity(PLAINTEXT_HEADER_LEN + body.len());
    write_i64(&mut plaintext, salt);
    write_i64(&mut plaintext, session_id);
    write_i64(&mut plaintext, msg_id);
    write_i32(&mut plaintext, seq_no);
    plaintext.extend_from_slice(body);

    let sealed = mtproto2::seal(auth_key, direction, &plaintext)?;

    let mut frame = Vec::with_capacity(HEADER_LEN + sealed.ciphertext.len());
    write_i64(&mut frame, auth_key_id);
    frame.extend_from_slice(&sealed.msg_key);
    frame.extend_from_slice(&sealed.ciphertext);

    if frame.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge {
            what: "encrypted frame",
            len: frame.len(),
            max: MAX_FRAME_LEN,
        });
    }
    Ok(frame)
}

/// Расшифровывает и разбирает кадр.
///
/// `expected_auth_key_id` должен совпадать с `auth_key_id` из кадра:
/// это защищает от подстановки кадра из другой сессии/ключа.
pub fn open_frame(
    auth_key: &AuthKey,
    direction: Direction,
    expected_auth_key_id: i64,
    frame: &[u8],
) -> Result<OpenedFrame, ProtocolError> {
    if frame.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge {
            what: "encrypted frame",
            len: frame.len(),
            max: MAX_FRAME_LEN,
        });
    }
    if frame.len() < HEADER_LEN + MIN_CIPHERTEXT_LEN {
        return Err(ProtocolError::Truncated {
            needed: HEADER_LEN + MIN_CIPHERTEXT_LEN,
            available: frame.len(),
        });
    }

    let mut reader = Reader::new(frame);
    let auth_key_id = reader.read_i64()?;
    if auth_key_id != expected_auth_key_id {
        return Err(ProtocolError::Malformed(format!(
            "auth_key_id mismatch: got {auth_key_id}, expected {expected_auth_key_id}"
        )));
    }

    let mut msg_key = [0u8; MSG_KEY_LEN];
    msg_key.copy_from_slice(reader.take(MSG_KEY_LEN)?);
    let ciphertext = reader.take(reader.remaining())?;

    let plaintext = mtproto2::open(auth_key, direction, &msg_key, ciphertext)?;
    if plaintext.len() < PLAINTEXT_HEADER_LEN {
        return Err(ProtocolError::Malformed(format!(
            "decrypted message is too short: {} bytes",
            plaintext.len()
        )));
    }

    let mut plain_reader = Reader::new(&plaintext);
    let salt = plain_reader.read_i64()?;
    let session_id = plain_reader.read_i64()?;
    let msg_id = plain_reader.read_i64()?;
    let seq_no = plain_reader.read_i32()?;
    let payload = plaintext[PLAINTEXT_HEADER_LEN..].to_vec();

    Ok(OpenedFrame {
        auth_key_id,
        salt,
        session_id,
        msg_id,
        seq_no,
        payload,
    })
}

/// Проверяет, что остаток payload после TL-тела — допустимый паддинг.
pub fn check_payload_padding(payload_len: usize, body_len: usize) -> Result<usize, ProtocolError> {
    Ok(mtproto2::check_padding(payload_len, body_len)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_auth_key() -> AuthKey {
        let mut key = [0u8; mtproto2::AUTH_KEY_LEN];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = ((i * 13 + 5) % 256) as u8;
        }
        key
    }

    fn test_body() -> Vec<u8> {
        let mut body = Vec::new();
        write_i32(&mut body, 0x1234_5678u32 as i32);
        write_i64(&mut body, 0xDEAD_BEEF);
        body
    }

    #[test]
    fn plain_frame_roundtrip() {
        let body = test_body();
        let frame = build_plain_frame(123, &body).unwrap();
        let parsed = parse_plain_frame(&frame).unwrap();
        assert_eq!(parsed.msg_id, 123);
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn plain_frame_requires_zero_auth_key_id_and_exact_length() {
        let mut frame = build_plain_frame(1, &[0, 0, 0, 0]).unwrap();
        frame[0] = 1;
        assert!(parse_plain_frame(&frame).is_err());

        let mut frame = build_plain_frame(1, &[0, 0, 0, 0]).unwrap();
        frame.push(0);
        assert!(parse_plain_frame(&frame).is_err());
    }

    #[test]
    fn encrypted_frame_roundtrip_both_directions() {
        let auth_key = test_auth_key();
        let body = test_body();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);

        for direction in [Direction::ClientToServer, Direction::ServerToClient] {
            let frame =
                seal_frame(&auth_key, direction, auth_key_id, 555, 777, 999, 3, &body).unwrap();
            let opened = open_frame(&auth_key, direction, auth_key_id, &frame).unwrap();
            assert_eq!(opened.auth_key_id, auth_key_id);
            assert_eq!(opened.session_id, 555);
            assert_eq!(opened.salt, 777);
            assert_eq!(opened.msg_id, 999);
            assert_eq!(opened.seq_no, 3);
            assert_eq!(&opened.payload[..body.len()], &body[..]);
            let padding = check_payload_padding(opened.payload.len(), body.len()).unwrap();
            assert!((mtproto2::MIN_PADDING..=mtproto2::MAX_PADDING).contains(&padding));
        }
    }

    #[test]
    fn wrong_direction_key_cannot_open_frame() {
        let auth_key = test_auth_key();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);
        let frame = seal_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id,
            1,
            0,
            2,
            1,
            &test_body(),
        )
        .unwrap();
        assert!(open_frame(&auth_key, Direction::ServerToClient, auth_key_id, &frame).is_err());
    }

    #[test]
    fn wrong_auth_key_id_is_rejected() {
        let auth_key = test_auth_key();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);
        let frame = seal_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id,
            1,
            0,
            2,
            1,
            &test_body(),
        )
        .unwrap();
        assert!(open_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id ^ 1,
            &frame
        )
        .is_err());
    }

    #[test]
    fn tampered_frame_is_rejected() {
        let auth_key = test_auth_key();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);
        let mut frame = seal_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id,
            1,
            0,
            2,
            1,
            &test_body(),
        )
        .unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 0x01;
        assert!(open_frame(&auth_key, Direction::ClientToServer, auth_key_id, &frame).is_err());
    }

    #[test]
    fn short_frame_is_rejected() {
        let auth_key = test_auth_key();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);
        assert!(open_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id,
            &[0u8; 10]
        )
        .is_err());
    }

    #[test]
    fn oversized_body_is_rejected() {
        let auth_key = test_auth_key();
        let auth_key_id = mtproto2::auth_key_id(&auth_key);
        let body = vec![0u8; MAX_FRAME_LEN];
        assert!(seal_frame(
            &auth_key,
            Direction::ClientToServer,
            auth_key_id,
            1,
            0,
            2,
            1,
            &body
        )
        .is_err());
    }
}
