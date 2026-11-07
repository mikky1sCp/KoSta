use crate::error::Error;
use std::io::{Read, Write};

// Трейты сериализации TL
pub trait TlWrite {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error>;
}

pub trait TlRead: Sized {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error>;
}

pub trait TlBoxed: TlWrite {
    fn constructor_id(&self) -> u32;
}

// ----- Примитивные типы -----

impl TlWrite for i32 {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.to_le_bytes())?;
        Ok(())
    }
}
impl TlRead for i32 {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }
}

impl TlWrite for u32 {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.to_le_bytes())?;
        Ok(())
    }
}
impl TlRead for u32 {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}

impl TlWrite for i64 {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.to_le_bytes())?;
        Ok(())
    }
}
impl TlRead for i64 {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Int128(pub [u8; 16]);

impl TlWrite for Int128 {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.0)?;
        Ok(())
    }
}
impl TlRead for Int128 {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 16];
        reader.read_exact(&mut buf)?;
        Ok(Int128(buf))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Int256(pub [u8; 32]);

impl TlWrite for Int256 {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.0)?;
        Ok(())
    }
}
impl TlRead for Int256 {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf)?;
        Ok(Int256(buf))
    }
}

// TL-строка: длина + данные + padding до кратности 4
impl TlWrite for String {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        write_tl_bytes(self.as_bytes(), writer)
    }
}
impl TlRead for String {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let bytes = read_tl_bytes(reader)?;
        String::from_utf8(bytes).map_err(|e| Error::Custom(format!("Invalid UTF-8: {}", e)))
    }
}

impl TlWrite for Vec<u8> {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        write_tl_bytes(self, writer)
    }
}
impl TlRead for Vec<u8> {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        read_tl_bytes(reader)
    }
}

// Вспомогательные функции для TL-строк

/// Максимальная длина одного TL-поля (строка/bytes).
/// Транспорт ограничивает фрейм 1 МиБ; запас нужен для расшифрованных
/// внутренних данных, которые могут быть чуть больше.
pub const MAX_TL_BYTES: usize = 4 * 1024 * 1024;

/// Максимальное число элементов в TL-векторе.
pub const MAX_TL_VECTOR_ITEMS: usize = 100_000;

/// Максимальная длина, которую можно записать в 3-байтовый TL-заголовок.
const MAX_TL_BYTES_ON_WIRE: usize = 0xFF_FFFF;

/// Выравнивание TL-поля: и заголовок длины, и данные вместе
/// должны давать длину, кратную 4.
fn tl_padding(len: usize, header_len: usize) -> usize {
    (4 - (len + header_len) % 4) % 4
}

fn write_tl_bytes<W: Write>(data: &[u8], writer: &mut W) -> Result<(), Error> {
    let len = data.len();
    let header_len = if len <= 253 { 1 } else { 4 };
    if len <= 253 {
        writer.write_all(&[len as u8])?;
    } else {
        if len > MAX_TL_BYTES_ON_WIRE {
            return Err(Error::Custom(format!(
                "TL bytes too long to serialize: {} (max {})",
                len, MAX_TL_BYTES_ON_WIRE
            )));
        }
        writer.write_all(&[254u8])?;
        writer.write_all(&(len as u32).to_le_bytes()[..3])?;
    }
    writer.write_all(data)?;
    let padding = tl_padding(len, header_len);
    if padding != 0 {
        writer.write_all(&[0u8; 3][..padding])?;
    }
    Ok(())
}

fn read_tl_bytes<R: Read>(reader: &mut R) -> Result<Vec<u8>, Error> {
    let mut first_byte = [0u8; 1];
    reader.read_exact(&mut first_byte)?;
    let (len, header_len) = if first_byte[0] == 254 {
        let mut buf = [0u8; 3];
        reader.read_exact(&mut buf)?;
        (u32::from_le_bytes([buf[0], buf[1], buf[2], 0]) as usize, 4)
    } else {
        (first_byte[0] as usize, 1)
    };

    // Длину присылает удалённая сторона: без лимита `vec![0u8; len]`
    // позволял зарезервировать мегабайты по одному заголовку.
    if len > MAX_TL_BYTES {
        return Err(Error::Custom(format!(
            "TL bytes too long: {} (max {})",
            len, MAX_TL_BYTES
        )));
    }

    // Читаем ровно len байт, но буфер растёт только по мере реальных данных,
    // а не до заявленной длины заранее.
    let mut data = Vec::new();
    let read = reader.by_ref().take(len as u64).read_to_end(&mut data)?;
    if read != len {
        return Err(Error::UnexpectedEof);
    }

    let padding = tl_padding(len, header_len);
    if padding != 0 {
        let mut pad = [0u8; 3];
        reader.read_exact(&mut pad[..padding])?;
    }
    Ok(data)
}

// Вектор TL: количество элементов (i32) + элементы
impl<T: TlWrite> TlWrite for Vec<T> {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let len = i32::try_from(self.len())
            .map_err(|_| Error::Custom(format!("Too many vector items: {}", self.len())))?;
        len.write_bytes(writer)?;
        for item in self {
            item.write_bytes(writer)?;
        }
        Ok(())
    }
}
impl<T: TlRead> TlRead for Vec<T> {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let raw_count = i32::read_bytes(reader)?;
        // Отрицательное значение раньше приводилось к usize и давало
        // Vec::with_capacity(usize::MAX) — паника прямо из сетевого ввода.
        if raw_count < 0 {
            return Err(Error::Custom(format!(
                "Negative TL vector length: {}",
                raw_count
            )));
        }
        let count = raw_count as usize;
        if count > MAX_TL_VECTOR_ITEMS {
            return Err(Error::Custom(format!(
                "TL vector too long: {} (max {})",
                count, MAX_TL_VECTOR_ITEMS
            )));
        }

        // Предварительная аллокация ограничена: элементы читаются по одному,
        // поэтому память тратится только на реально пришедшие данные.
        let mut vec = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            vec.push(T::read_bytes(reader)?);
        }
        Ok(vec)
    }
}

impl TlWrite for bool {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        let val = if *self { 1i32 } else { 0i32 };
        val.write_bytes(writer)
    }
}
impl TlRead for bool {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let val = i32::read_bytes(reader)?;
        Ok(val != 0)
    }
}

// ----- НОВЫЕ ТИПЫ ДЛЯ ПРИКЛАДНЫХ СООБЩЕНИЙ -----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(pub i64);
impl TlWrite for MessageId {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.0.write_bytes(writer)
    }
}
impl TlRead for MessageId {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(MessageId(i64::read_bytes(reader)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatId(pub i64);
impl TlWrite for ChatId {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.0.write_bytes(writer)
    }
}
impl TlRead for ChatId {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(ChatId(i64::read_bytes(reader)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub i64);
impl TlWrite for UserId {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.0.write_bytes(writer)
    }
}
impl TlRead for UserId {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(UserId(i64::read_bytes(reader)?))
    }
}

// ===== НОВЫЙ ТИП ДЛЯ ДИАЛОГОВ =====
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogId(pub i64);
impl TlWrite for DialogId {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.0.write_bytes(writer)
    }
}
impl TlRead for DialogId {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(DialogId(i64::read_bytes(reader)?))
    }
}

// ===== РЕАЛИЗАЦИЯ ДЛЯ Option<T> =====
impl<T: TlWrite> TlWrite for Option<T> {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        match self {
            Some(val) => {
                1i32.write_bytes(writer)?;
                val.write_bytes(writer)?;
            }
            None => {
                0i32.write_bytes(writer)?;
            }
        }
        Ok(())
    }
}
impl<T: TlRead> TlRead for Option<T> {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let exists = i32::read_bytes(reader)?;
        if exists != 0 {
            Ok(Some(T::read_bytes(reader)?))
        } else {
            Ok(None)
        }
    }
}

// ===== ОСТАВШИЕСЯ ТИПЫ (UserStatus) =====
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Online,
    Offline,
    Typing,
    Recently,
    LastWeek,
    LastMonth,
}
impl TlWrite for UserStatus {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        let code = match self {
            UserStatus::Online => 0,
            UserStatus::Offline => 1,
            UserStatus::Typing => 2,
            UserStatus::Recently => 3,
            UserStatus::LastWeek => 4,
            UserStatus::LastMonth => 5,
        };
        code.write_bytes(writer)
    }
}
impl TlRead for UserStatus {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let code = i32::read_bytes(reader)?;
        match code {
            0 => Ok(UserStatus::Online),
            1 => Ok(UserStatus::Offline),
            2 => Ok(UserStatus::Typing),
            3 => Ok(UserStatus::Recently),
            4 => Ok(UserStatus::LastWeek),
            5 => Ok(UserStatus::LastMonth),
            _ => Err(Error::Custom("Invalid UserStatus code".into())),
        }
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    use crate::tl::constructors::{HistoryResult, ResPQ, TlObject};
    use std::io::Cursor;

    fn sample_res_pq() -> TlObject {
        TlObject::ResPQ(ResPQ {
            nonce: Int128([1; 16]),
            server_nonce: Int128([2; 16]),
            pq: vec![0x11; 8],
            p: vec![0x22; 4],
            q: vec![0x33; 4],
            server_public_key_fingerprints: vec![7],
        })
    }

    #[test]
    fn negative_vector_length_is_rejected() {
        let mut buf = Vec::new();
        (-1i32).write_bytes(&mut buf).unwrap();
        // Раньше `as usize` давал usize::MAX и Vec::with_capacity паниковал.
        assert!(Vec::<i64>::read_bytes(&mut Cursor::new(&buf)).is_err());
    }

    #[test]
    fn oversized_vector_length_is_rejected() {
        let mut buf = Vec::new();
        i32::MAX.write_bytes(&mut buf).unwrap();
        assert!(Vec::<i64>::read_bytes(&mut Cursor::new(&buf)).is_err());
    }

    #[test]
    fn vector_count_without_payload_is_eof() {
        let mut buf = Vec::new();
        1000i32.write_bytes(&mut buf).unwrap();
        assert!(Vec::<i64>::read_bytes(&mut Cursor::new(&buf)).is_err());
    }

    #[test]
    fn oversized_tl_bytes_length_is_rejected() {
        // 0xFFFFFF = 16 МиБ — больше лимита, полезной нагрузки нет
        let buf = vec![254u8, 0xFF, 0xFF, 0xFF];
        assert!(Vec::<u8>::read_bytes(&mut Cursor::new(&buf)).is_err());
    }

    #[test]
    fn roundtrip_boundary_lengths() {
        for len in [0usize, 1, 3, 253, 254, 255, 1024] {
            let data = vec![0xABu8; len];
            let mut buf = Vec::new();
            data.write_bytes(&mut buf).unwrap();
            let decoded = Vec::<u8>::read_bytes(&mut Cursor::new(&buf)).unwrap();
            assert_eq!(data, decoded, "len={}", len);
            assert_eq!(buf.len() % 4, 0, "TL-поле должно быть выровнено по 4 байта");
        }
    }

    #[test]
    fn forged_message_count_is_rejected() {
        let obj = TlObject::HistoryResult(HistoryResult {
            messages: vec![],
            total_count: 0,
        });
        let mut buf = Vec::new();
        obj.write_boxed(&mut buf).unwrap();

        for forged in [-1i32, i32::MAX] {
            let mut broken = buf.clone();
            broken[4..8].copy_from_slice(&forged.to_le_bytes());
            assert!(
                TlObject::read_boxed(&mut Cursor::new(&broken)).is_err(),
                "count={} должен давать ошибку, а не панику",
                forged
            );
        }
    }

    #[test]
    fn truncated_input_never_panics() {
        let mut buf = Vec::new();
        sample_res_pq().write_boxed(&mut buf).unwrap();
        for cut in 0..=buf.len() {
            let _ = TlObject::read_boxed(&mut Cursor::new(&buf[..cut]));
        }
    }

    #[test]
    fn random_garbage_never_panics() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let len = (next() % 96) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xFF) as u8).collect();
            let _ = TlObject::read_boxed(&mut Cursor::new(&bytes));
        }
    }
}