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
fn write_tl_bytes<W: Write>(data: &[u8], writer: &mut W) -> Result<(), Error> {
    let len = data.len();
    if len <= 253 {
        writer.write_all(&[len as u8])?;
    } else {
        writer.write_all(&[254u8])?;
        writer.write_all(&(len as u32).to_le_bytes()[..3])?;
    }
    writer.write_all(data)?;
    let padding = (4 - (len % 4)) % 4;
    if padding != 0 {
        writer.write_all(&vec![0u8; padding])?;
    }
    Ok(())
}

fn read_tl_bytes<R: Read>(reader: &mut R) -> Result<Vec<u8>, Error> {
    let mut first_byte = [0u8; 1];
    reader.read_exact(&mut first_byte)?;
    let len = if first_byte[0] == 254 {
        let mut buf = [0u8; 3];
        reader.read_exact(&mut buf)?;
        u32::from_le_bytes([buf[0], buf[1], buf[2], 0]) as usize
    } else {
        first_byte[0] as usize
    };
    let mut data = vec![0u8; len];
    reader.read_exact(&mut data)?;
    let padding = (4 - (len % 4)) % 4;
    if padding != 0 {
        let mut pad = vec![0u8; padding];
        reader.read_exact(&mut pad)?;
    }
    Ok(data)
}

// Вектор TL: количество элементов (i32) + элементы
impl<T: TlWrite> TlWrite for Vec<T> {
    fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        (self.len() as i32).write_bytes(writer)?;
        for item in self {
            item.write_bytes(writer)?;
        }
        Ok(())
    }
}
impl<T: TlRead> TlRead for Vec<T> {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let count = i32::read_bytes(reader)? as usize;
        let mut vec = Vec::with_capacity(count);
        for _ in 0..count {
            vec.push(T::read_bytes(reader)?);
        }
        Ok(vec)
    }
}

// В types.rs добавить:
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