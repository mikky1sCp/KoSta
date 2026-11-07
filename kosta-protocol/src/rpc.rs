//! `rpc_result` / `rpc_error`: корреляция запросов и ответов.
//!
//! `rpc_error` всегда вложен в `rpc_result` как результат:
//!
//! ```text
//! rpc_result#f35c6d01 req_msg_id:long result:Object
//! rpc_error#2144ca19 error_code:int error_message:string
//! ```

use crate::error::ProtocolError;
use crate::tl::{write_bytes, write_i32, write_i64, Reader};

/// Идентификатор `rpc_result#f35c6d01`.
pub const RPC_RESULT_ID: u32 = 0xf35c_6d01;
/// Идентификатор `rpc_error#2144ca19`.
pub const RPC_ERROR_ID: u32 = 0x2144_ca19;
/// Максимальная длина текста RPC-ошибки.
pub const MAX_RPC_ERROR_MESSAGE: usize = 4096;

/// Прикладная RPC-ошибка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// Числовой код ошибки.
    pub code: i32,
    /// Текст ошибки.
    pub message: String,
}

/// Результат RPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcOutcome {
    /// Boxed TL-объект результата (с его constructor id).
    Ok(Vec<u8>),
    /// Прикладная ошибка.
    Err(RpcError),
}

/// Собирает `rpc_result` с уже сериализованным boxed-результатом.
pub fn build_rpc_result(req_msg_id: i64, result_body: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if result_body.len() < 4 {
        return Err(ProtocolError::Malformed(
            "rpc_result payload must be a boxed TL object".into(),
        ));
    }
    let mut out = Vec::with_capacity(4 + 8 + result_body.len());
    write_i32(&mut out, RPC_RESULT_ID as i32);
    write_i64(&mut out, req_msg_id);
    out.extend_from_slice(result_body);
    Ok(out)
}

/// Собирает `rpc_result` с вложенным `rpc_error`.
pub fn build_rpc_error(
    req_msg_id: i64,
    code: i32,
    message: &str,
) -> Result<Vec<u8>, ProtocolError> {
    if message.len() > MAX_RPC_ERROR_MESSAGE {
        return Err(ProtocolError::TooLarge {
            what: "rpc_error message",
            len: message.len(),
            max: MAX_RPC_ERROR_MESSAGE,
        });
    }
    let mut error = Vec::new();
    write_i32(&mut error, RPC_ERROR_ID as i32);
    write_i32(&mut error, code);
    write_bytes(&mut error, message.as_bytes())?;
    build_rpc_result(req_msg_id, &error)
}

/// Разбирает `rpc_result` и определяет, пришёл результат или ошибка.
pub fn parse_rpc_result(body: &[u8]) -> Result<(i64, RpcOutcome), ProtocolError> {
    let mut reader = Reader::new(body);
    let id = reader.read_constructor_id()?;
    if id != RPC_RESULT_ID {
        return Err(ProtocolError::Malformed(format!(
            "expected rpc_result 0x{RPC_RESULT_ID:08x}, got 0x{id:08x}"
        )));
    }
    let req_msg_id = reader.read_i64()?;

    let result_id = reader.read_constructor_id()?;
    if result_id == RPC_ERROR_ID {
        let code = reader.read_i32()?;
        let message = reader.read_string()?;
        return Ok((req_msg_id, RpcOutcome::Err(RpcError { code, message })));
    }

    let mut result = Vec::with_capacity(4 + reader.remaining());
    write_i32(&mut result, result_id as i32);
    result.extend_from_slice(reader.take(reader.remaining())?);
    Ok((req_msg_id, RpcOutcome::Ok(result)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_result_object() -> Vec<u8> {
        let mut result = Vec::new();
        write_i32(&mut result, 0x1234_5678u32 as i32);
        write_i64(&mut result, 42);
        result
    }

    #[test]
    fn rpc_result_ok_roundtrip() {
        let body = build_rpc_result(777, &fake_result_object()).unwrap();
        let (req_msg_id, outcome) = parse_rpc_result(&body).unwrap();
        assert_eq!(req_msg_id, 777);
        assert_eq!(outcome, RpcOutcome::Ok(fake_result_object()));
    }

    #[test]
    fn rpc_error_roundtrip() {
        let body = build_rpc_error(777, 400, "bad request").unwrap();
        let (req_msg_id, outcome) = parse_rpc_result(&body).unwrap();
        assert_eq!(req_msg_id, 777);
        assert_eq!(
            outcome,
            RpcOutcome::Err(RpcError {
                code: 400,
                message: "bad request".into(),
            })
        );
    }

    #[test]
    fn empty_result_is_rejected() {
        assert!(build_rpc_result(1, &[]).is_err());
        assert!(build_rpc_result(1, &[0, 0, 0]).is_err());
    }

    #[test]
    fn oversized_error_message_is_rejected() {
        let message = "x".repeat(MAX_RPC_ERROR_MESSAGE + 1);
        assert!(build_rpc_error(1, 400, &message).is_err());
    }

    #[test]
    fn malformed_rpc_result_is_rejected() {
        let mut body = build_rpc_result(1, &fake_result_object()).unwrap();
        body[0] ^= 0xFF;
        assert!(parse_rpc_result(&body).is_err());

        let body = build_rpc_result(1, &fake_result_object()).unwrap();
        assert!(parse_rpc_result(&body[..6]).is_err());
    }
}
