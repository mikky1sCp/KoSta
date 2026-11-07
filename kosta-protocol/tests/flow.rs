//! Интеграционный сценарий «клиент ↔ сервер» на уровне MTProto 2.0:
//! кадры, salt, контейнер, acks, `rpc_result`, повтор запроса.

use kosta_crypto::mtproto2::{self, AuthKey, Direction};
use kosta_protocol::tl::write_i32;
use kosta_protocol::{ack, container, frame, msg_id, rpc, salt};

const NOW: i64 = 1_800_000_000;
const SERVER_SALT: i64 = 0x5A5A_1234_5678_9ABC;

fn test_auth_key() -> AuthKey {
    let mut key = [0u8; mtproto2::AUTH_KEY_LEN];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = ((i * 29 + 17) % 256) as u8;
    }
    key
}

/// Псевдо-TL-объект: constructor id + полезная нагрузка.
fn boxed_body(constructor: u32, payload: i64) -> Vec<u8> {
    let mut body = Vec::new();
    write_i32(&mut body, constructor as i32);
    body.extend_from_slice(&payload.to_le_bytes());
    body
}

#[test]
fn client_server_exchange_with_salt_ack_and_rpc() {
    let auth_key = test_auth_key();
    let auth_key_id = mtproto2::auth_key_id(&auth_key);
    let session_id: i64 = 0x0123_4567_89AB_CDEF;

    let mut client_salt: i64 = 0;
    let mut server_salt = salt::ServerSalt::new(0);

    let mut client_ids = msg_id::MsgIdGenerator::client();
    let mut server_ids = msg_id::MsgIdGenerator::server();
    let mut client_seq = msg_id::SeqNoGenerator::new();

    let mut server_validator = msg_id::MsgIdValidator::client();
    let mut client_validator = msg_id::MsgIdValidator::server();

    let mut server_acks = ack::AckTracker::new();
    let mut response_cache = ack::ResponseCache::new(8);

    // --- 1. Клиент отправляет content-related запрос с salt = 0. -----------
    let request_msg_id = client_ids.next(NOW);
    let request_body = boxed_body(0x1111_1111, 7);
    let request_frame = frame::seal_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        session_id,
        client_salt,
        request_msg_id,
        client_seq.next_content(),
        &request_body,
    )
    .unwrap();

    let opened = frame::open_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        &request_frame,
    )
    .unwrap();
    assert_eq!(opened.session_id, session_id);
    assert_eq!(opened.msg_id, request_msg_id);
    assert_eq!(opened.seq_no, 1);
    assert!(
        server_salt.is_acceptable(opened.salt),
        "initial salt 0 must be accepted"
    );
    assert_eq!(&opened.payload[..request_body.len()], &request_body[..]);
    frame::check_payload_padding(opened.payload.len(), request_body.len()).unwrap();

    server_validator.validate(opened.msg_id, NOW).unwrap();

    // --- 2. Сервер отвечает контейнером: new_session_created + rpc_result,
    //        плюс копит ack на запрос клиента. ---------------------------
    server_acks.mark(opened.msg_id);
    let ack_body = server_acks.build().unwrap();
    let new_session_body = salt::build_new_session_created(opened.msg_id, session_id, SERVER_SALT);
    let response_body = boxed_body(0x2222_2222, 42);
    let rpc_body = rpc::build_rpc_result(opened.msg_id, &response_body).unwrap();
    response_cache.insert(opened.msg_id, rpc_body.clone());

    let mut server_seq = msg_id::SeqNoGenerator::new();
    let response_messages = vec![
        container::ContainerMessage {
            msg_id: server_ids.next(NOW),
            seq_no: 0,
            body: ack_body,
        },
        container::ContainerMessage {
            msg_id: server_ids.next(NOW),
            seq_no: 0,
            body: new_session_body,
        },
        container::ContainerMessage {
            msg_id: server_ids.next(NOW),
            seq_no: server_seq.next_content(),
            body: rpc_body.clone(),
        },
    ];
    let container_body = container::build_container(&response_messages).unwrap();

    let container_frame = frame::seal_frame(
        &auth_key,
        Direction::ServerToClient,
        auth_key_id,
        session_id,
        0,
        server_ids.next(NOW),
        0,
        &container_body,
    )
    .unwrap();

    // --- 3. Клиент разбирает контейнер и обновляет salt. -------------------
    let opened = frame::open_frame(
        &auth_key,
        Direction::ServerToClient,
        auth_key_id,
        &container_frame,
    )
    .unwrap();
    let messages = container::parse_container(&opened.payload[..container_body.len()]).unwrap();
    assert_eq!(messages.len(), 3);

    // Внутренние сообщения контейнера сгенерированы раньше его самого,
    // поэтому сначала валидируем их, затем msg_id конверта.
    let mut got_ack = false;
    let mut got_salt = false;
    let mut got_rpc = false;
    for message in &messages {
        client_validator.validate(message.msg_id, NOW).unwrap();

        if let Ok(acked_ids) = ack::parse_msgs_ack(&message.body) {
            assert_eq!(acked_ids, vec![request_msg_id]);
            got_ack = true;
        } else if let Ok(created) = salt::parse_new_session_created(&message.body) {
            assert_eq!(created.server_salt, SERVER_SALT);
            assert_eq!(created.first_msg_id, request_msg_id);
            client_salt = created.server_salt;
            server_salt.update(created.server_salt);
            got_salt = true;
        } else if let Ok((req_msg_id, outcome)) = rpc::parse_rpc_result(&message.body) {
            assert_eq!(req_msg_id, request_msg_id);
            assert_eq!(outcome, rpc::RpcOutcome::Ok(response_body.clone()));
            got_rpc = true;
        }
    }
    assert!(
        got_ack && got_salt && got_rpc,
        "all three response messages must be parsed"
    );
    client_validator.validate(opened.msg_id, NOW).unwrap();
    server_acks.drain();

    // --- 4. Клиент шлёт следующий запрос уже с новым salt. -----------------
    let next_request_id = client_ids.next(NOW);
    let next_frame = frame::seal_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        session_id,
        client_salt,
        next_request_id,
        client_seq.next_content(),
        &boxed_body(0x1111_1111, 8),
    )
    .unwrap();
    let opened_next = frame::open_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        &next_frame,
    )
    .unwrap();
    assert!(
        server_salt.is_acceptable(opened_next.salt),
        "new salt must be accepted"
    );
    server_validator.validate(opened_next.msg_id, NOW).unwrap();

    // --- 5. Повтор запроса: дубль и сохранённый ответ. ---------------------
    let duplicate = server_validator.validate(request_msg_id, NOW);
    assert!(matches!(
        duplicate,
        Err(kosta_protocol::ProtocolError::Duplicate)
    ));
    assert_eq!(
        response_cache.get(request_msg_id),
        Some(rpc_body.as_slice())
    );
}

#[test]
fn tampered_frame_from_server_is_rejected_by_client() {
    let auth_key = test_auth_key();
    let auth_key_id = mtproto2::auth_key_id(&auth_key);
    let mut ids = msg_id::MsgIdGenerator::server();
    let frame = frame::seal_frame(
        &auth_key,
        Direction::ServerToClient,
        auth_key_id,
        7,
        0,
        ids.next(NOW),
        1,
        &boxed_body(0x3333_3333, 1),
    )
    .unwrap();

    let mut tampered = frame.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x80;
    assert!(
        frame::open_frame(&auth_key, Direction::ServerToClient, auth_key_id, &tampered).is_err()
    );
    assert!(frame::open_frame(&auth_key, Direction::ServerToClient, auth_key_id, &frame).is_ok());
}

#[test]
fn container_with_client_messages_is_accepted() {
    let auth_key = test_auth_key();
    let auth_key_id = mtproto2::auth_key_id(&auth_key);
    let mut ids = msg_id::MsgIdGenerator::client();

    let messages = vec![
        container::ContainerMessage {
            msg_id: ids.next(NOW),
            seq_no: 1,
            body: boxed_body(0x4444_4444, 1),
        },
        container::ContainerMessage {
            msg_id: ids.next(NOW),
            seq_no: 3,
            body: boxed_body(0x5555_5555, 2),
        },
    ];
    let body = container::build_container(&messages).unwrap();
    let mut validator = msg_id::MsgIdValidator::client();
    for message in &messages {
        validator.validate(message.msg_id, NOW).unwrap();
    }

    let frame = frame::seal_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        1,
        0,
        ids.next(NOW),
        0,
        &body,
    )
    .unwrap();
    let opened =
        frame::open_frame(&auth_key, Direction::ClientToServer, auth_key_id, &frame).unwrap();
    assert_eq!(
        container::parse_container(&opened.payload[..body.len()]).unwrap(),
        messages
    );
}
