//! MTProto 2.0 message layer для KoSta.
//!
//! Крейт отвечает за «внутренний» слой протокола поверх шифрования
//! `kosta-crypto`:
//!
//! * кадры — [`frame`] (`auth_key_id`/`msg_key`/AES-256-IGE и незашифрованный
//!   конверт хендшейка);
//! * `msg_id`/`seq_no` — [`msg_id`] (время, чётность, окно, монотонность);
//! * контейнеры — [`container`] (`msg_container#73f1f8dc`);
//! * подтверждения и кэш ответов — [`ack`] (`msgs_ack`);
//! * RPC — [`rpc`] (`rpc_result`/`rpc_error`);
//! * серверный salt — [`salt`] (`new_session_created`);
//! * каноническое TL-кодирование — [`tl`].
//!
//! Прикладные TL-объекты на этом уровне непрозрачны: `Vec<u8>` с
//! boxed-представлением (constructor id + тело). Типизированный TL и dispatch
//! появятся в крейте `kosta-tl` (этап 3 миграции).

pub mod ack;
pub mod container;
pub mod error;
pub mod frame;
pub mod msg_id;
pub mod rpc;
pub mod salt;
pub mod tl;

pub use error::ProtocolError;
