//! Стандартный MTProto-abridged транспорт (init-байт `0xef`).
//!
//! Отличие от легаси-модуля [`crate::abridged`]: длина кадра кодируется в
//! 4-байтовых словах, а не в байтах, и соединение начинается с байта `0xef`.
//! Quick-ack (0xef в начале кадра) не поддерживается — для KoSta он не нужен.
//!
//! Формат:
//! ```text
//! [0xef]                          // один раз в начале соединения
//! words = payload_len / 4
//! words < 0x7f  -> [words: u8]
//! иначе          -> [0x7f][words: 3 bytes LE]
//! [payload]
//! ```

use std::io::{Read, Write};

use crate::error::TransportError;

/// Байт инициализации abridged-транспорта.
pub const INIT_BYTE: u8 = 0xEF;
/// Максимальный размер payload (совпадает с лимитом легаси-транспорта).
pub const MAX_FRAME_SIZE: usize = 1_048_576;

/// Пишет init-байт `0xef` в начало соединения.
pub fn write_init<W: Write>(writer: &mut W) -> Result<(), TransportError> {
    writer.write_all(&[INIT_BYTE])?;
    writer.flush()?;
    Ok(())
}

/// Читает и проверяет init-байт `0xef`.
pub fn read_init<R: Read>(reader: &mut R) -> Result<(), TransportError> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    if byte[0] != INIT_BYTE {
        return Err(TransportError::Custom(format!(
            "expected MTProto abridged init byte 0xef, got 0x{:02x}",
            byte[0]
        )));
    }
    Ok(())
}

/// Кодирует payload в MTProto-abridged кадр (без init-байта).
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, TransportError> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(TransportError::Custom(format!(
            "frame too large: {} bytes (max {})",
            payload.len(),
            MAX_FRAME_SIZE
        )));
    }
    if !payload.len().is_multiple_of(4) {
        return Err(TransportError::Custom(format!(
            "abridged payload length must be a multiple of 4, got {}",
            payload.len()
        )));
    }

    let words = payload.len() / 4;
    let mut frame = Vec::with_capacity(4 + payload.len());
    if words < 0x7f {
        frame.push(words as u8);
    } else {
        if words > 0xFF_FFFF {
            return Err(TransportError::Custom(format!(
                "abridged frame too large: {} words",
                words
            )));
        }
        frame.push(0x7f);
        frame.extend_from_slice(&(words as u32).to_le_bytes()[..3]);
    }
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Читает один MTProto-abridged кадр из потока.
pub fn decode_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first)?;
    let words = match first[0] {
        small @ 0..=0x7e => small as usize,
        0x7f => {
            let mut buf = [0u8; 3];
            reader.read_exact(&mut buf)?;
            u32::from_le_bytes([buf[0], buf[1], buf[2], 0]) as usize
        }
        other => {
            return Err(TransportError::Custom(format!(
                "unsupported MTProto abridged length byte 0x{other:02x}"
            )))
        }
    };

    let len = words * 4;
    if len > MAX_FRAME_SIZE {
        return Err(TransportError::Custom(format!(
            "frame too large: {} bytes (max {})",
            len, MAX_FRAME_SIZE
        )));
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// Декодирует кадр из начала буфера (для тестов и mock-транспорта).
pub fn decode_frame_from_slice(buffer: &mut Vec<u8>) -> Result<Vec<u8>, TransportError> {
    if buffer.is_empty() {
        return Err(TransportError::Custom("no data".into()));
    }
    let (words, header_len) = match buffer[0] {
        small @ 0..=0x7e => (small as usize, 1),
        0x7f => {
            if buffer.len() < 4 {
                return Err(TransportError::Custom("incomplete length header".into()));
            }
            let words = u32::from_le_bytes([buffer[1], buffer[2], buffer[3], 0]) as usize;
            (words, 4)
        }
        other => {
            return Err(TransportError::Custom(format!(
                "unsupported MTProto abridged length byte 0x{other:02x}"
            )))
        }
    };

    let len = words * 4;
    if len > MAX_FRAME_SIZE {
        return Err(TransportError::Custom(format!(
            "frame too large: {} bytes (max {})",
            len, MAX_FRAME_SIZE
        )));
    }
    let total = header_len + len;
    if buffer.len() < total {
        return Err(TransportError::Custom("incomplete frame".into()));
    }
    let payload = buffer[header_len..total].to_vec();
    buffer.drain(..total);
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Детерминированный xorshift (без внешних зависимостей).
    struct Xorshift(u64);
    impl Xorshift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| (self.next() & 0xff) as u8).collect()
        }
    }

    #[test]
    fn random_and_truncated_bytes_never_panic() {
        let mut rng = Xorshift(0xC0FF_EE11_2233_4455);
        for _ in 0..250 {
            let len = (rng.next() % 96) as usize;
            let mut buffer = rng.bytes(len);
            // slice-декодер мутирует буфер; главное — Ok или Err, не паника.
            let _ = decode_frame_from_slice(&mut buffer);
            let _ = decode_frame(&mut Cursor::new(rng.bytes(len)));
        }

        let frame = encode_frame(&[0xABu8; 64]).unwrap();
        for cut in 0..=frame.len() {
            let mut slice = frame[..cut].to_vec();
            let _ = decode_frame_from_slice(&mut slice);
            let _ = decode_frame(&mut Cursor::new(&frame[..cut]));
        }
    }

    #[test]
    fn init_byte_roundtrip() {
        let mut buffer = Vec::new();
        write_init(&mut buffer).unwrap();
        assert_eq!(buffer, vec![INIT_BYTE]);
        read_init(&mut Cursor::new(buffer)).unwrap();
    }

    #[test]
    fn wrong_init_byte_is_rejected() {
        assert!(read_init(&mut Cursor::new(vec![0xEE])).is_err());
        assert!(read_init(&mut Cursor::new(Vec::new())).is_err());
    }

    #[test]
    fn frame_roundtrip_at_boundaries() {
        for words in [0usize, 1, 126, 127, 128, 1000] {
            let payload: Vec<u8> = (0..words * 4).map(|i| (i % 251) as u8).collect();
            let frame = encode_frame(&payload).unwrap();
            let decoded = decode_frame(&mut Cursor::new(frame)).unwrap();
            assert_eq!(decoded, payload, "words={words}");
        }
    }

    #[test]
    fn short_and_long_length_encodings_differ() {
        let short = encode_frame(&vec![0u8; 4 * 126]).unwrap();
        assert_eq!(short[0], 126);
        assert_eq!(short.len(), 1 + 4 * 126);

        let long = encode_frame(&vec![0u8; 4 * 127]).unwrap();
        assert_eq!(long[0], 0x7f);
        assert_eq!(&long[1..4], &[127, 0, 0]);
        assert_eq!(long.len(), 4 + 4 * 127);
    }

    #[test]
    fn non_multiple_of_four_is_rejected() {
        assert!(encode_frame(&[1, 2, 3]).is_err());
        assert!(encode_frame(&[1]).is_err());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let payload = vec![0u8; MAX_FRAME_SIZE + 4];
        assert!(encode_frame(&payload).is_err());
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let frame = encode_frame(&[7u8; 64]).unwrap();
        assert!(decode_frame(&mut Cursor::new(&frame[..10])).is_err());
    }

    #[test]
    fn slice_decoder_consumes_one_frame() {
        let mut buffer = encode_frame(&[1u8; 8]).unwrap();
        buffer.extend_from_slice(&encode_frame(&[2u8; 8]).unwrap());
        let first = decode_frame_from_slice(&mut buffer).unwrap();
        assert_eq!(first, vec![1u8; 8]);
        let second = decode_frame_from_slice(&mut buffer).unwrap();
        assert_eq!(second, vec![2u8; 8]);
        assert!(buffer.is_empty());
    }
}
