//! Минимальное каноническое TL-кодирование, нужное слою MTProto 2.0:
//! примитивы, `bytes`/`string`, векторы, boxed-объекты.
//!
//! Прикладные TL-объекты на этом слое непрозрачны (это `Vec<u8>` с
//! boxed-представлением). Полноценные типы и dispatch появятся в крейте
//! `kosta-tl` (этап 3), а здесь живут только мета-конструкции MTProto
//! (`msg_container`, `msgs_ack`, `rpc_result`, …).

use crate::error::ProtocolError;

/// Максимальный размер одного TL-поля `bytes`/`string`.
pub const MAX_TL_BYTES: usize = 4 * 1024 * 1024;
/// Максимальное число элементов вектора.
pub const MAX_TL_VECTOR_ITEMS: usize = 100_000;
/// Идентификатор `vector#1cb5c415`.
pub const VECTOR_ID: u32 = 0x1cb5c415;
/// Идентификатор `true#997275b5`.
pub const TRUE_ID: u32 = 0x997275b5;
/// Идентификатор `false#bc799737`.
pub const FALSE_ID: u32 = 0xbc799737;

/// Записывает `int` (little-endian).
pub fn write_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Записывает `long` (little-endian).
pub fn write_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Записывает TL-`bytes`/`string`: 1 или 4 байта длины + данные + выравнивание.
pub fn write_bytes(out: &mut Vec<u8>, data: &[u8]) -> Result<(), ProtocolError> {
    if data.len() > MAX_TL_BYTES {
        return Err(ProtocolError::TooLarge {
            what: "TL bytes",
            len: data.len(),
            max: MAX_TL_BYTES,
        });
    }

    let header_len = if data.len() <= 253 { 1 } else { 4 };
    if header_len == 1 {
        out.push(data.len() as u8);
    } else {
        out.push(0xFE);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
    }
    out.extend_from_slice(data);

    let padding = (4 - (data.len() + header_len) % 4) % 4;
    out.extend_from_slice(&[0u8; 3][..padding]);
    Ok(())
}

/// Записывает TL-`string` (UTF-8).
pub fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), ProtocolError> {
    write_bytes(out, value.as_bytes())
}

/// Записывает заголовок TL-вектора: id `0x1cb5c415` + количество.
pub fn write_vector_len(out: &mut Vec<u8>, len: usize) -> Result<(), ProtocolError> {
    if len > MAX_TL_VECTOR_ITEMS {
        return Err(ProtocolError::TooLarge {
            what: "TL vector",
            len,
            max: MAX_TL_VECTOR_ITEMS,
        });
    }
    write_i32(out, VECTOR_ID as i32);
    write_i32(out, len as i32);
    Ok(())
}

/// Читатель TL-данных с проверкой границ.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Создаёт читатель над `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// Текущая позиция.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Сколько байт осталось.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Все ли данные прочитаны.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Отрезает `len` байт и сдвигает позицию.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        if self.remaining() < len {
            return Err(ProtocolError::Truncated {
                needed: len,
                available: self.remaining(),
            });
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..self.pos])
    }

    /// Пропускает `len` байт (например, паддинг).
    pub fn skip(&mut self, len: usize) -> Result<(), ProtocolError> {
        self.take(len)?;
        Ok(())
    }

    /// Читает `int`.
    pub fn read_i32(&mut self) -> Result<i32, ProtocolError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    /// Читает `long`.
    pub fn read_i64(&mut self) -> Result<i64, ProtocolError> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    /// Читает `uint`/constructor id.
    pub fn read_constructor_id(&mut self) -> Result<u32, ProtocolError> {
        Ok(self.read_i32()? as u32)
    }

    /// Читает TL-`bytes`.
    pub fn read_bytes(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let first = self.take(1)?[0];
        let (len, header_len) = if first == 0xFE {
            let raw = self.take(3)?;
            let len = u32::from_le_bytes([raw[0], raw[1], raw[2], 0]) as usize;
            (len, 4)
        } else {
            (first as usize, 1)
        };
        if len > MAX_TL_BYTES {
            return Err(ProtocolError::TooLarge {
                what: "TL bytes",
                len,
                max: MAX_TL_BYTES,
            });
        }

        let data = self.take(len)?.to_vec();
        let padding = (4 - (len + header_len) % 4) % 4;
        self.skip(padding)?;
        Ok(data)
    }

    /// Читает TL-`string`.
    pub fn read_string(&mut self) -> Result<String, ProtocolError> {
        let bytes = self.read_bytes()?;
        String::from_utf8(bytes)
            .map_err(|e| ProtocolError::Malformed(format!("invalid UTF-8 string: {e}")))
    }

    /// Читает заголовок TL-вектора и возвращает количество элементов.
    pub fn read_vector_len(&mut self) -> Result<usize, ProtocolError> {
        let id = self.read_constructor_id()?;
        if id != VECTOR_ID {
            return Err(ProtocolError::Malformed(format!(
                "expected vector id 0x{VECTOR_ID:08x}, got 0x{id:08x}"
            )));
        }
        let count = self.read_i32()?;
        if count < 0 {
            return Err(ProtocolError::Malformed(format!(
                "negative TL vector length: {count}"
            )));
        }
        let count = count as usize;
        if count > MAX_TL_VECTOR_ITEMS {
            return Err(ProtocolError::TooLarge {
                what: "TL vector",
                len: count,
                max: MAX_TL_VECTOR_ITEMS,
            });
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_roundtrip_including_254_boundary() {
        for len in [0usize, 1, 2, 3, 4, 253, 254, 255, 1024] {
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut out = Vec::new();
            write_bytes(&mut out, &data).unwrap();
            assert!(out.len().is_multiple_of(4), "len={len} out={}", out.len());

            let mut reader = Reader::new(&out);
            assert_eq!(reader.read_bytes().unwrap(), data);
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn strings_roundtrip_unicode() {
        let mut out = Vec::new();
        write_string(&mut out, "привет, KoSta 👋").unwrap();
        let mut reader = Reader::new(&out);
        assert_eq!(reader.read_string().unwrap(), "привет, KoSta 👋");
    }

    #[test]
    fn vector_header_roundtrip() {
        let mut out = Vec::new();
        write_vector_len(&mut out, 3).unwrap();
        write_i64(&mut out, 10);
        write_i64(&mut out, 20);
        write_i64(&mut out, 30);

        let mut reader = Reader::new(&out);
        assert_eq!(reader.read_vector_len().unwrap(), 3);
        assert_eq!(reader.read_i64().unwrap(), 10);
        assert_eq!(reader.read_i64().unwrap(), 20);
        assert_eq!(reader.read_i64().unwrap(), 30);
        assert!(reader.is_empty());
    }

    #[test]
    fn truncated_input_is_rejected() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert!(matches!(
            reader.read_i32(),
            Err(ProtocolError::Truncated { .. })
        ));

        let mut reader = Reader::new(&[5, 1, 2]);
        assert!(reader.read_bytes().is_err());
    }

    #[test]
    fn oversized_vector_and_bytes_are_rejected() {
        let mut out = Vec::new();
        assert!(write_vector_len(&mut out, MAX_TL_VECTOR_ITEMS + 1).is_err());
        assert!(write_bytes(&mut out, &vec![0u8; MAX_TL_BYTES + 1]).is_err());
    }

    #[test]
    fn bad_vector_id_is_rejected() {
        let mut out = Vec::new();
        write_i32(&mut out, 0x1234_5678u32 as i32);
        write_i32(&mut out, 1);
        let mut reader = Reader::new(&out);
        assert!(reader.read_vector_len().is_err());
    }
}
