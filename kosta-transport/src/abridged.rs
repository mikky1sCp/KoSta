// =============================================================================
// kosta-transport/src/abridged.rs
// =============================================================================
use crate::error::TransportError;
use std::io::Read;

const MAX_FRAME_SIZE: usize = 1_048_576; // 1 MiB

/// Сериализует payload в Abridged‑фрейм.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    if len <= 0x7f {
        let mut frame = Vec::with_capacity(1 + len);
        frame.push(len as u8);
        frame.extend_from_slice(payload);
        frame
    } else {
        assert!(len <= 0xffffff, "payload too large for Abridged transport");
        let mut frame = Vec::with_capacity(4 + len);
        frame.push(0x7f);
        frame.extend_from_slice(&(len as u32).to_le_bytes()[..3]);
        frame.extend_from_slice(payload);
        frame
    }
}

/// Декодирует один Abridged‑фрейм из потока, реализующего `Read`.
pub fn decode_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut len_byte = [0u8; 1];
    reader.read_exact(&mut len_byte)?;
    let len = match len_byte[0] {
        l @ 0..=0x7e => l as usize,
        0x7f => {
            let mut buf = [0u8; 3];
            reader.read_exact(&mut buf)?;
            let raw = u32::from_le_bytes([buf[0], buf[1], buf[2], 0]);
            raw as usize
        }
        other => {
            return Err(TransportError::Custom(format!(
                "Unsupported Abridged length byte: 0x{:02x}",
                other
            )))
        }
    };
    if len > MAX_FRAME_SIZE {
        return Err(TransportError::Custom(format!(
            "Frame too large: {} bytes (max {} bytes)",
            len, MAX_FRAME_SIZE
        )));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// Декодирует один Abridged‑фрейм из начала `Vec<u8>` и удаляет прочитанные байты.
/// Используется `MockTransport`.
pub fn decode_frame_from_slice(buf: &mut Vec<u8>) -> Result<Vec<u8>, TransportError> {
    if buf.is_empty() {
        return Err(TransportError::Custom("No data".into()));
    }
    let first = buf[0];
    let (payload_len, header_len) = match first {
        l @ 0..=0x7e => (l as usize, 1),
        0x7f => {
            if buf.len() < 4 {
                return Err(TransportError::Custom("Incomplete length header".into()));
            }
            let len_bytes = [buf[1], buf[2], buf[3], 0];
            let len = u32::from_le_bytes(len_bytes) as usize;
            (len, 4)
        }
        other => {
            return Err(TransportError::Custom(format!(
                "Unsupported Abridged length byte: 0x{:02x}",
                other
            )))
        }
    };

    if payload_len > MAX_FRAME_SIZE {
        return Err(TransportError::Custom(format!(
            "Frame too large: {} bytes (max {} bytes)",
            payload_len, MAX_FRAME_SIZE
        )));
    }

    let total = header_len + payload_len;
    if buf.len() < total {
        return Err(TransportError::Custom("Incomplete frame".into()));
    }

    let payload = buf[header_len..total].to_vec();
    buf.drain(..total);
    Ok(payload)
}