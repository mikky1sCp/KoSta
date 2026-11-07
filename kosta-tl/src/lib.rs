//! Типизированный TL-слой KoSta, сгенерированный из `schema/kosta.tl`.
//!
//! Крейт не зависит от транспорта и протокола: только каноническое TL-кодирование
//! ([`tl`]) и сгенерированные типы ([`schema`]). Прикладные объекты описаны в
//! схеме; id конструкторов без явного `#id` считаются как CRC32 от объявления.
//!
//! ```no_run
//! use kosta_tl::{Int128, Reader, ReqPq, TlRead, TlWrite};
//!
//! let request = ReqPq {
//!     nonce: Int128([1u8; 16]),
//! };
//! let mut bytes = Vec::new();
//! request.write_bytes(&mut bytes).unwrap();
//!
//! let parsed = ReqPq::read_bytes(&mut Reader::new(&bytes)).unwrap();
//! assert_eq!(parsed, request);
//! ```

pub mod schema;
pub mod tl;

pub use schema::*;
pub use tl::{
    Bytes, Int128, Int256, Reader, TlError, TlRead, TlWrite, FALSE_ID, MAX_TL_BYTES,
    MAX_TL_VECTOR_ITEMS, TRUE_ID, VECTOR_ID,
};
