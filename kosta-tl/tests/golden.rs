//! Golden-тесты TL: для объектов хендшейка зафиксированы побайтовые векторы,
//! снятые с легаси-реализации `kosta-core` перед её удалением (этап 6). Это
//! защищает от случайного изменения порядка полей или кодирования.

use kosta_tl::{
    Bytes, ChatMessage, ClientDhInnerData, DialogInfo, DialogList, HistoryResult, Int128, Int256,
    Message, MsgContainer, PqInnerData, Reader, ReqDhParams, ReqPq, RpcResult, ServerDhInnerData,
    ServerDhParams, SetClientDhParams, SetClientDhParamsAnswer, TlError, TlObject, TlRead, TlWrite,
};

fn hex(source: &str) -> Vec<u8> {
    assert!(source.len().is_multiple_of(2), "odd hex length");
    (0..source.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&source[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn ours<T: TlWrite>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.write_bytes(&mut out).expect("serialize");
    out
}

fn read_ours<T: TlRead>(bytes: &[u8]) -> T {
    T::read_bytes(&mut Reader::new(bytes)).expect("deserialize")
}

fn int128(value: u8) -> Int128 {
    Int128([value; 16])
}

fn int256(value: u8) -> Int256 {
    Int256([value; 32])
}

// Векторы сняты с kosta-core до его удаления (этап 6).
const REQ_PQ_HEX: &str = "7897466001010101010101010101010101010101";
const PQ_INNER_DATA_HEX: &str = "ec5ac98302aabb0001cc000001dd000004040404040404040404040404040404050505050505050505050505050505050606060606060606060606060606060606060606060606060606060606060606";
const REQ_DH_PARAMS_HEX: &str = "bee412d707070707070707070707070707070707080808080808080808080808080808080101000001020000785634120000000002aabb00";
const SERVER_DH_PARAMS_OK_HEX: &str = "4104e6d0090909090909090909090909090909090a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0211220002334400";
const SERVER_DH_INNER_DATA_HEX: &str = "ba0d89b50b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0200000001ff000001aa000039300000";
const CLIENT_DH_INNER_DATA_HEX: &str = "54b643660d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e000000000000000001bb0000";
const SET_CLIENT_DH_PARAMS_HEX: &str = "1f5f04f50f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f1010101010101010101010101010101002556600";
const DH_GEN_OK_HEX: &str = "34f7cb3b111111111111111111111111111111111212121212121212121212121212121213131313131313131313131313131313";

#[test]
fn req_pq_golden() {
    let object = ReqPq {
        nonce: int128(1),
    };
    let expected = hex(REQ_PQ_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<ReqPq>(&expected), object);
}

#[test]
fn pq_inner_data_golden() {
    let object = PqInnerData {
        pq: Bytes(vec![0xAA, 0xBB]),
        p: Bytes(vec![0xCC]),
        q: Bytes(vec![0xDD]),
        nonce: int128(4),
        server_nonce: int128(5),
        new_nonce: int256(6),
    };
    let expected = hex(PQ_INNER_DATA_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<PqInnerData>(&expected), object);
}

#[test]
fn req_dh_params_golden() {
    let object = ReqDhParams {
        nonce: int128(7),
        server_nonce: int128(8),
        p: Bytes(vec![0x01]),
        q: Bytes(vec![0x02]),
        public_key_fingerprint: 0x1234_5678,
        encrypted_data: Bytes(vec![0xAA, 0xBB]),
    };
    let expected = hex(REQ_DH_PARAMS_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<ReqDhParams>(&expected), object);
}

#[test]
fn server_dh_params_ok_golden() {
    let object = ServerDhParams::ServerDhParamsOk {
        nonce: int128(9),
        server_nonce: int128(10),
        encrypted_answer: Bytes(vec![0x11, 0x22]),
        signature: Bytes(vec![0x33, 0x44]),
    };
    let expected = hex(SERVER_DH_PARAMS_OK_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<ServerDhParams>(&expected), object);
}

#[test]
fn server_dh_inner_data_golden() {
    let object = ServerDhInnerData {
        nonce: int128(11),
        server_nonce: int128(12),
        g: 2,
        dh_prime: Bytes(vec![0xFF]),
        g_a: Bytes(vec![0xAA]),
        server_time: 12_345,
    };
    let expected = hex(SERVER_DH_INNER_DATA_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<ServerDhInnerData>(&expected), object);
}

#[test]
fn client_dh_inner_data_golden() {
    let object = ClientDhInnerData {
        nonce: int128(13),
        server_nonce: int128(14),
        retry_id: 0,
        g_b: Bytes(vec![0xBB]),
    };
    let expected = hex(CLIENT_DH_INNER_DATA_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<ClientDhInnerData>(&expected), object);
}

#[test]
fn set_client_dh_params_golden() {
    let object = SetClientDhParams {
        nonce: int128(15),
        server_nonce: int128(16),
        encrypted_data: Bytes(vec![0x55, 0x66]),
    };
    let expected = hex(SET_CLIENT_DH_PARAMS_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<SetClientDhParams>(&expected), object);
}

#[test]
fn dh_gen_ok_golden() {
    let object = SetClientDhParamsAnswer::DhGenOk {
        nonce: int128(17),
        server_nonce: int128(18),
        new_nonce_hash1: int128(19),
    };
    let expected = hex(DH_GEN_OK_HEX);
    assert_eq!(ours(&object), expected);
    assert_eq!(read_ours::<SetClientDhParamsAnswer>(&expected), object);
}

fn sample_message() -> ChatMessage {
    ChatMessage::ChatMessage {
        id: 105,
        chat_id: 12,
        sender_id: 8,
        text: "привет, KoSta".to_string(),
        timestamp: 1_800_000_000,
        is_outgoing: false,
        read: true,
        delivered: false,
    }
}

#[test]
fn app_objects_roundtrip() {
    let message = sample_message();
    assert_eq!(read_ours::<ChatMessage>(&ours(&message)), message);

    let empty: ChatMessage = ChatMessage::ChatMessageEmpty;
    assert_eq!(read_ours::<ChatMessage>(&ours(&empty)), empty);

    let dialog = DialogInfo {
        id: 12,
        title: "+79990000000".to_string(),
        is_group: false,
        participants: vec![7, 8],
        last_message: message.clone(),
        unread_count: 2,
    };
    assert_eq!(read_ours::<DialogInfo>(&ours(&dialog)), dialog);

    let list = DialogList {
        dialogs: vec![dialog.clone(), dialog],
    };
    assert_eq!(read_ours::<DialogList>(&ours(&list)), list);

    let history = HistoryResult {
        messages: vec![message.clone()],
        total_count: 1,
    };
    assert_eq!(read_ours::<HistoryResult>(&ours(&history)), history);

    let rpc = RpcResult {
        req_msg_id: 99,
        result: Box::new(TlObject::ChatMessage(message)),
    };
    assert_eq!(read_ours::<RpcResult>(&ours(&rpc)), rpc);
}

#[test]
fn container_and_envelope_roundtrip() {
    let message = Message {
        msg_id: 1_800_000_000_000_000_001,
        seqno: 1,
        bytes: 8,
        body: Bytes(vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]),
    };
    let container = MsgContainer {
        messages: vec![message.clone(), message],
    };
    assert_eq!(read_ours::<MsgContainer>(&ours(&container)), container);
}

#[test]
fn tl_object_dispatch_and_unknown_id() {
    let message = sample_message();
    let bytes = ours(&message);
    let object = TlObject::read_boxed(&mut Reader::new(&bytes)).unwrap();
    assert!(matches!(
        object,
        TlObject::ChatMessage(ChatMessage::ChatMessage { .. })
    ));

    let unknown = vec![0xDE, 0xAD, 0xBE, 0xEF];
    assert!(matches!(
        TlObject::read_boxed(&mut Reader::new(&unknown)),
        Err(TlError::UnknownConstructor(0xEFBE_ADDE))
    ));

    let truncated = vec![0xDE, 0xAD, 0xBE];
    assert!(TlObject::read_boxed(&mut Reader::new(&truncated)).is_err());
}