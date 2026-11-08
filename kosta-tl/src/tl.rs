//! Примитивы TL: сериализация, `Reader`, `Bytes`, `Int128`/`Int256`, ошибки.
//!
//! Кодирование каноническое: `bytes`/`string` — 1- или 4-байтовый заголовок
//! длины + данные + выравнивание до 4; `vector` — id `0x1cb5c415` + count;
//! `Bool` — конструкторы `true#997275b5` / `false#bc799737`.

use std::fmt;

/// Максимальный размер одного `bytes`/`string`-поля.
pub const MAX_TL_BYTES: usize = 4 * 1024 * 1024;
/// Максимальное число элементов вектора.
pub const MAX_TL_VECTOR_ITEMS: usize = 100_000;
/// Идентификатор `vector#1cb5c415`.
pub const VECTOR_ID: u32 = 0x1cb5_c415;
/// Идентификатор `true#997275b5`.
pub const TRUE_ID: u32 = 0x9972_75b5;
/// Идентификатор `false#bc799737`.
pub const FALSE_ID: u32 = 0xbc79_9737;

/// Ошибка TL-сериализации.
#[derive(Debug)]
pub enum TlError {
    /// Структурно некорректные данные.
    Malformed(String),
    /// Недостаточно данных.
    Truncated { needed: usize, available: usize },
    /// Превышен лимит размера.
    TooLarge {
        what: &'static str,
        len: usize,
        max: usize,
    },
    /// Неизвестный constructor id.
    UnknownConstructor(u32),
    /// Некорректный UTF-8 в строке.
    InvalidUtf8,
}

impl fmt::Display for TlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TlError::Malformed(message) => write!(f, "malformed TL: {message}"),
            TlError::Truncated { needed, available } => {
                write!(f, "truncated TL: need {needed} bytes, have {available}")
            }
            TlError::TooLarge { what, len, max } => {
                write!(f, "{what} is too large: {len} (max {max})")
            }
            TlError::UnknownConstructor(id) => {
                write!(f, "unknown TL constructor 0x{id:08x}")
            }
            TlError::InvalidUtf8 => write!(f, "invalid UTF-8 in TL string"),
        }
    }
}

impl std::error::Error for TlError {}

/// TL-значение, которое умеет писать себя в буфер.
pub trait TlWrite {
    /// Пишет боксованный объект (constructor id + поля) или примитив.
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError>;
}

/// TL-значение, которое умеет читать себя из [`Reader`].
pub trait TlRead: Sized {
    /// Читает боксованный объект (constructor id + поля) или примитив.
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError>;
}

/// TL-`bytes`: в отличие от `vector<u8>`, кодируется как длина + данные.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    /// Пустые байты.
    pub fn empty() -> Self {
        Bytes(Vec::new())
    }

    /// Длина в байтах.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Пусто ли поле.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// TL-`int128`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Int128(pub [u8; 16]);

/// TL-`int256`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Int256(pub [u8; 32]);

/// Записывает `int`.
pub fn write_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Записывает `long`.
pub fn write_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Записывает `double`.
pub fn write_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Записывает TL-`bytes`/`string`.
pub fn write_bytes(out: &mut Vec<u8>, data: &[u8]) -> Result<(), TlError> {
    if data.len() > MAX_TL_BYTES {
        return Err(TlError::TooLarge {
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

/// Записывает заголовок `vector#1cb5c415` с количеством элементов.
pub fn write_vector_len(out: &mut Vec<u8>, len: usize) -> Result<(), TlError> {
    if len > MAX_TL_VECTOR_ITEMS {
        return Err(TlError::TooLarge {
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
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], TlError> {
        if self.remaining() < len {
            return Err(TlError::Truncated {
                needed: len,
                available: self.remaining(),
            });
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..self.pos])
    }

    /// Пропускает `len` байт (паддинг).
    pub fn skip(&mut self, len: usize) -> Result<(), TlError> {
        self.take(len)?;
        Ok(())
    }

    /// Читает `int` как `u32` (constructor id).
    pub fn read_u32(&mut self) -> Result<u32, TlError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    /// Читает `int`.
    pub fn read_i32(&mut self) -> Result<i32, TlError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    /// Читает `long`.
    pub fn read_i64(&mut self) -> Result<i64, TlError> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    /// Читает `double`.
    pub fn read_f64(&mut self) -> Result<f64, TlError> {
        let bytes = self.take(8)?;
        Ok(f64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    /// Читает TL-`bytes`.
    pub fn read_bytes_field(&mut self) -> Result<Bytes, TlError> {
        let first = self.take(1)?[0];
        let (len, header_len) = if first == 0xFE {
            let raw = self.take(3)?;
            (u32::from_le_bytes([raw[0], raw[1], raw[2], 0]) as usize, 4)
        } else {
            (first as usize, 1)
        };
        if len > MAX_TL_BYTES {
            return Err(TlError::TooLarge {
                what: "TL bytes",
                len,
                max: MAX_TL_BYTES,
            });
        }
        let data = self.take(len)?.to_vec();
        let padding = (4 - (len + header_len) % 4) % 4;
        self.skip(padding)?;
        Ok(Bytes(data))
    }

    /// Читает TL-`string`.
    pub fn read_string(&mut self) -> Result<String, TlError> {
        let bytes = self.read_bytes_field()?;
        String::from_utf8(bytes.0).map_err(|_| TlError::InvalidUtf8)
    }

    /// Читает заголовок `vector` и возвращает количество элементов.
    pub fn read_vector_len(&mut self) -> Result<usize, TlError> {
        let id = self.read_u32()?;
        if id != VECTOR_ID {
            return Err(TlError::Malformed(format!(
                "expected vector id 0x{VECTOR_ID:08x}, got 0x{id:08x}"
            )));
        }
        let count = self.read_i32()?;
        if count < 0 {
            return Err(TlError::Malformed(format!(
                "negative TL vector length: {count}"
            )));
        }
        let count = count as usize;
        if count > MAX_TL_VECTOR_ITEMS {
            return Err(TlError::TooLarge {
                what: "TL vector",
                len: count,
                max: MAX_TL_VECTOR_ITEMS,
            });
        }
        Ok(count)
    }
}

impl TlWrite for i32 {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_i32(out, *self);
        Ok(())
    }
}

impl TlRead for i32 {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        reader.read_i32()
    }
}

impl TlWrite for i64 {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_i64(out, *self);
        Ok(())
    }
}

impl TlRead for i64 {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        reader.read_i64()
    }
}

impl TlWrite for f64 {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_f64(out, *self);
        Ok(())
    }
}

impl TlRead for f64 {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        reader.read_f64()
    }
}

impl TlWrite for bool {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        let id = if *self { TRUE_ID } else { FALSE_ID };
        write_i32(out, id as i32);
        Ok(())
    }
}

impl TlRead for bool {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        match reader.read_u32()? {
            TRUE_ID => Ok(true),
            FALSE_ID => Ok(false),
            other => Err(TlError::Malformed(format!(
                "expected Bool constructor, got 0x{other:08x}"
            ))),
        }
    }
}

impl TlWrite for String {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_bytes(out, self.as_bytes())
    }
}

impl TlRead for String {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        reader.read_string()
    }
}

impl TlWrite for Bytes {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_bytes(out, &self.0)
    }
}

impl TlRead for Bytes {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        reader.read_bytes_field()
    }
}

impl TlWrite for Int128 {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        out.extend_from_slice(&self.0);
        Ok(())
    }
}

impl TlRead for Int128 {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        let mut value = [0u8; 16];
        value.copy_from_slice(reader.take(16)?);
        Ok(Int128(value))
    }
}

impl TlWrite for Int256 {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        out.extend_from_slice(&self.0);
        Ok(())
    }
}

impl TlRead for Int256 {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        let mut value = [0u8; 32];
        value.copy_from_slice(reader.take(32)?);
        Ok(Int256(value))
    }
}

impl<T: TlWrite> TlWrite for Vec<T> {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        write_vector_len(out, self.len())?;
        for item in self {
            item.write_bytes(out)?;
        }
        Ok(())
    }
}

impl<T: TlRead> TlRead for Vec<T> {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        let count = reader.read_vector_len()?;
        let mut items = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            items.push(T::read_bytes(reader)?);
        }
        Ok(items)
    }
}

impl<T: TlWrite> TlWrite for Box<T> {
    fn write_bytes(&self, out: &mut Vec<u8>) -> Result<(), TlError> {
        (**self).write_bytes(out)
    }
}

impl<T: TlRead> TlRead for Box<T> {
    fn read_bytes(reader: &mut Reader<'_>) -> Result<Self, TlError> {
        Ok(Box::new(T::read_bytes(reader)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_roundtrip() {
        let mut out = Vec::new();
        (-42i32).write_bytes(&mut out).unwrap();
        i64::MAX.write_bytes(&mut out).unwrap();
        3.5f64.write_bytes(&mut out).unwrap();
        true.write_bytes(&mut out).unwrap();
        false.write_bytes(&mut out).unwrap();

        let mut reader = Reader::new(&out);
        assert_eq!(i32::read_bytes(&mut reader).unwrap(), -42);
        assert_eq!(i64::read_bytes(&mut reader).unwrap(), i64::MAX);
        assert_eq!(f64::read_bytes(&mut reader).unwrap(), 3.5);
        assert!(bool::read_bytes(&mut reader).unwrap());
        assert!(!bool::read_bytes(&mut reader).unwrap());
        assert!(reader.is_empty());
    }

    #[test]
    fn bytes_and_strings_roundtrip() {
        for len in [0usize, 1, 253, 254, 1024] {
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut out = Vec::new();
            Bytes(data.clone()).write_bytes(&mut out).unwrap();
            let mut reader = Reader::new(&out);
            assert_eq!(Bytes::read_bytes(&mut reader).unwrap().0, data);
            assert!(reader.is_empty());
        }

        let mut out = Vec::new();
        "привет 👋".to_string().write_bytes(&mut out).unwrap();
        let mut reader = Reader::new(&out);
        assert_eq!(String::read_bytes(&mut reader).unwrap(), "привет 👋");
    }

    #[test]
    fn vectors_roundtrip() {
        let values = vec![1i64, -2, 3, i64::MAX];
        let mut out = Vec::new();
        values.write_bytes(&mut out).unwrap();
        let mut reader = Reader::new(&out);
        assert_eq!(Vec::<i64>::read_bytes(&mut reader).unwrap(), values);
    }

    #[test]
    fn int128_and_int256_roundtrip() {
        let value = Int128([7u8; 16]);
        let mut out = Vec::new();
        value.write_bytes(&mut out).unwrap();
        let mut reader = Reader::new(&out);
        assert_eq!(Int128::read_bytes(&mut reader).unwrap(), value);

        let value = Int256([9u8; 32]);
        let mut out = Vec::new();
        value.write_bytes(&mut out).unwrap();
        let mut reader = Reader::new(&out);
        assert_eq!(Int256::read_bytes(&mut reader).unwrap(), value);
    }

    #[test]
    fn invalid_bool_constructor_is_rejected() {
        let mut out = Vec::new();
        write_i32(&mut out, 0x1234_5678u32 as i32);
        let mut reader = Reader::new(&out);
        assert!(bool::read_bytes(&mut reader).is_err());
    }

    #[test]
    fn truncation_is_reported() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert!(matches!(
            reader.read_i32(),
            Err(TlError::Truncated {
                needed: 4,
                available: 3
            })
        ));
    }

    #[test]
    fn oversized_values_are_rejected() {
        let mut out = Vec::new();
        assert!(write_vector_len(&mut out, MAX_TL_VECTOR_ITEMS + 1).is_err());
        assert!(write_bytes(&mut out, &vec![0u8; MAX_TL_BYTES + 1]).is_err());
    }

    #[test]
    fn bad_vector_id_is_rejected() {
        let mut out = Vec::new();
        write_i32(&mut out, 0x1111_1111);
        write_i32(&mut out, 1);
        let mut reader = Reader::new(&out);
        assert!(Vec::<i64>::read_bytes(&mut reader).is_err());
    }

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
        let mut rng = Xorshift(0xDEAD_BEEF_CAFE_F00D);

        // Случайные данные любой длины — только Ok/Err, не паника, для всех типов.
        for _ in 0..200 {
            let len = (rng.next() % 56) as usize;
            let data = rng.bytes(len);
            let mut r = Reader::new(&data);
            let _ = i32::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = i64::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = f64::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = bool::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = String::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = Bytes::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = Int128::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = Int256::read_bytes(&mut r);
            let mut r = Reader::new(&data);
            let _ = Vec::<i64>::read_bytes(&mut r);
        }

        // Каждый обрезанный префикс валидно-закодированного объекта тоже.
        let mut out = Vec::new();
        Bytes(vec![1, 2, 3, 4, 5]).write_bytes(&mut out).unwrap();
        vec![1i64, 2, 3].write_bytes(&mut out).unwrap();
        "coast".to_string().write_bytes(&mut out).unwrap();
        for cut in 0..out.len() {
            let mut r = Reader::new(&out[..cut]);
            let _ = Bytes::read_bytes(&mut r);
            let _ = Vec::<i64>::read_bytes(&mut r);
            let _ = String::read_bytes(&mut r);
        }
    }
}
