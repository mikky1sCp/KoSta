pub mod error;
pub mod session;
pub mod factor;
pub mod server_keys;
pub mod dh_checks;

pub use session::Session;
pub use error::KostaError;

#[cfg(test)]
mod tests {
    use super::*;
    use kosta_core::tl::constructors::{ReqPq, ResPQ, TlObject};
    use kosta_core::tl::types::Int128;
    use kosta_transport::mock::MockTransport;

    #[test]
    fn client_sends_req_pq_and_receives_res_pq() {
        // Клиент
        let client_mock = MockTransport::new();
        let mut session = Session::from_mock_transport(client_mock);
        let nonce = Int128([1u8; 16]);
        let req = ReqPq { nonce: nonce.clone() };
        session.send_tl(&TlObject::ReqPq(req)).unwrap();

        // Забираем отправленные клиентом данные и передаём их "серверу"
        let sent = session.transport.take_sent();

        let mut server_mock = MockTransport::new();
        server_mock.add_incoming(&sent);
        let mut server_session = Session::from_mock_transport(server_mock);

        // Сервер читает запрос
        let req_received = server_session.recv_tl().unwrap();
        match req_received {
            TlObject::ReqPq(req) => {
                // Формируем ответ
                let response = ResPQ {
                    nonce: req.nonce,
                    server_nonce: Int128([2u8; 16]),
                    pq: vec![0x17, 0xED, 0x48, 0x91, 0x41],
                    server_public_key_fingerprints: vec![0x12345678ABCDEF],
                };
                server_session.send_tl(&TlObject::ResPQ(response)).unwrap();

                // Забираем ответ сервера
                let reply = server_session.transport.take_sent();

                // Передаём ответ клиенту
                session.transport.add_incoming(&reply);

                // Клиент читает ответ
                let resp = session.recv_tl().unwrap();
                match resp {
                    TlObject::ResPQ(res_pq) => assert_eq!(res_pq.nonce, nonce),
                    _ => panic!("Expected ResPQ"),
                }
            }
            _ => panic!("Expected ReqPq"),
        }
    }
}