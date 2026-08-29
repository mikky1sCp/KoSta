pub mod error;
pub mod tl;

pub use error::Error;
pub use tl::*;

#[cfg(test)]
mod serialization_tests {
    use crate::tl::constructors::*;
    use crate::tl::types::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_all_constructors() {
        // ReqPq
        let req = ReqPq { nonce: Int128([1; 16]) };
        assert_roundtrip(TlObject::ReqPq(req));

        // ResPQ
        let res = ResPQ {
            nonce: Int128([2; 16]),
            server_nonce: Int128([3; 16]),
            pq: vec![0x11, 0x22],
            server_public_key_fingerprints: vec![0x12345678, 0x9abcdef0],
        };
        assert_roundtrip(TlObject::ResPQ(res));

        // PqInnerData
        let inner = PqInnerData {
            pq: vec![0xAA, 0xBB],
            p: vec![0xCC],
            q: vec![0xDD],
            nonce: Int128([4; 16]),
            server_nonce: Int128([5; 16]),
            new_nonce: Int256([6; 32]),
        };
        assert_roundtrip(TlObject::PqInnerData(inner));

        // ReqDHParams
        let req_dh = ReqDHParams {
            nonce: Int128([7; 16]),
            server_nonce: Int128([8; 16]),
            p: vec![0x01],
            q: vec![0x02],
            public_key_fingerprint: 0x12345678,
            encrypted_data: vec![0xAA, 0xBB],
        };
        assert_roundtrip(TlObject::ReqDHParams(req_dh));

        // ServerDHParamsOk
        let srv_dh_ok = ServerDHParamsOk {
            nonce: Int128([9; 16]),
            server_nonce: Int128([10; 16]),
            encrypted_answer: vec![0x11, 0x22],
            signature: vec![0x33, 0x44],
        };
        assert_roundtrip(TlObject::ServerDHParamsOk(srv_dh_ok));

        // ServerDHInnerData
        let srv_inner = ServerDHInnerData {
            nonce: Int128([11; 16]),
            server_nonce: Int128([12; 16]),
            g: 2,
            dh_prime: vec![0xFF],
            g_a: vec![0xAA],
            server_time: 12345,
        };
        assert_roundtrip(TlObject::ServerDHInnerData(srv_inner));

        // ClientDHInnerData
        let client_inner = ClientDHInnerData {
            nonce: Int128([13; 16]),
            server_nonce: Int128([14; 16]),
            retry_id: 0,
            g_b: vec![0xBB],
        };
        assert_roundtrip(TlObject::ClientDHInnerData(client_inner));

        // SetClientDHParams
        let set_dh = SetClientDHParams {
            nonce: Int128([15; 16]),
            server_nonce: Int128([16; 16]),
            encrypted_data: vec![0xCC, 0xDD],
        };
        assert_roundtrip(TlObject::SetClientDHParams(set_dh));

        // DHGenOk
        let dh_gen = DHGenOk {
            nonce: Int128([17; 16]),
            server_nonce: Int128([18; 16]),
            new_nonce_hash1: Int128([19; 16]),
        };
        assert_roundtrip(TlObject::DHGenOk(dh_gen));

        // --- Прикладные ---
        let message = Message {
            id: MessageId(1),
            chat_id: ChatId(2),
            sender_id: UserId(3),
            text: "Hello".to_string(),
            timestamp: 123456,
            is_outgoing: true,
            read: false,
            delivered: false,
        };
        assert_roundtrip(TlObject::Message(message));

        let chat = Chat {
            id: ChatId(1),
            title: "Chat".to_string(),
            participants: vec![UserId(1), UserId(2)],
            is_group: false,
        };
        assert_roundtrip(TlObject::Chat(chat));

        let send_msg = SendMessage {
            chat_id: ChatId(1),
            text: "Hi".to_string(),
            random_id: 42,
        };
        assert_roundtrip(TlObject::SendMessage(send_msg));

        let ack = SendMessageAck {
            message_id: MessageId(5),
        };
        assert_roundtrip(TlObject::SendMessageAck(ack));

        let get_hist = GetHistory {
            chat_id: ChatId(1),
            offset: 0,
            limit: 10,
        };
        assert_roundtrip(TlObject::GetHistory(get_hist));

        let hist_res = HistoryResult {
            messages: vec![],
            total_count: 0,
        };
        assert_roundtrip(TlObject::HistoryResult(hist_res));

        let status = UserStatusUpdate {
            user_id: UserId(1),
            status: UserStatus::Online,
        };
        assert_roundtrip(TlObject::UserStatusUpdate(status));

        let signup = SignUp {
            phone: "+123".to_string(),
            password: "pass".to_string(),
        };
        assert_roundtrip(TlObject::SignUp(signup));
    }

    fn assert_roundtrip(obj: TlObject) {
        let mut buf = Vec::new();
        obj.write_boxed(&mut buf).unwrap();
        let mut cursor = Cursor::new(&buf);
        let decoded = TlObject::read_boxed(&mut cursor).unwrap();
        match (obj.clone(), decoded.clone()) {
            (TlObject::ReqPq(a), TlObject::ReqPq(b)) => assert_eq!(a, b),
            (TlObject::ResPQ(a), TlObject::ResPQ(b)) => assert_eq!(a, b),
            (TlObject::PqInnerData(a), TlObject::PqInnerData(b)) => assert_eq!(a, b),
            (TlObject::ReqDHParams(a), TlObject::ReqDHParams(b)) => assert_eq!(a, b),
            (TlObject::ServerDHParamsOk(a), TlObject::ServerDHParamsOk(b)) => assert_eq!(a, b),
            (TlObject::ServerDHInnerData(a), TlObject::ServerDHInnerData(b)) => assert_eq!(a, b),
            (TlObject::ClientDHInnerData(a), TlObject::ClientDHInnerData(b)) => assert_eq!(a, b),
            (TlObject::SetClientDHParams(a), TlObject::SetClientDHParams(b)) => assert_eq!(a, b),
            (TlObject::DHGenOk(a), TlObject::DHGenOk(b)) => assert_eq!(a, b),
            (TlObject::Message(a), TlObject::Message(b)) => assert_eq!(a, b),
            (TlObject::Chat(a), TlObject::Chat(b)) => assert_eq!(a, b),
            (TlObject::SendMessage(a), TlObject::SendMessage(b)) => assert_eq!(a, b),
            (TlObject::SendMessageAck(a), TlObject::SendMessageAck(b)) => assert_eq!(a, b),
            (TlObject::GetHistory(a), TlObject::GetHistory(b)) => assert_eq!(a, b),
            (TlObject::HistoryResult(a), TlObject::HistoryResult(b)) => assert_eq!(a, b),
            (TlObject::UserStatusUpdate(a), TlObject::UserStatusUpdate(b)) => assert_eq!(a, b),
            (TlObject::SignUp(a), TlObject::SignUp(b)) => assert_eq!(a, b),
            _ => panic!("Mismatch: {:#?} vs {:#?}", obj.clone(), decoded.clone()),
        }
    }
}