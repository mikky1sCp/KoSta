//! Мусороустойчивость: случайные и обрезанные байты никогда не должны
//! приводить к панике ни в одном парсере (допустимы только `Ok` или `Err`).
//! Это кремневая защита от DoS: вместо паники соединение должно закрыться
//! ошибкой. Детерминированный PRNG, без внешних зависимостей.

use kosta_crypto::mtproto2::{self, AuthKey, Direction};
use kosta_protocol::{container, frame, rpc, salt};

/// Детерминированный xorshift-ГПСЧ.
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

fn test_auth_key() -> AuthKey {
    let mut key = [0u8; mtproto2::AUTH_KEY_LEN];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = ((i * 13 + 5) % 256) as u8;
    }
    key
}

#[test]
fn random_bytes_do_not_panic_in_any_parser() {
    let auth_key = test_auth_key();
    let auth_key_id = mtproto2::auth_key_id(&auth_key);

    let mut rng = Xorshift(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..200 {
        let len = (rng.next() as usize) % 768;
        let data = rng.bytes(len);

        // Любой результат допустим — главное, чтобы не было паники.
        let _ = frame::parse_plain_frame(&data);
        let _ = frame::open_frame(&auth_key, Direction::ClientToServer, auth_key_id, &data);
        let _ = container::parse_container(&data);
        let _ = container::is_container(&data);
        let _ = rpc::parse_rpc_result(&data);
        let _ = salt::parse_new_session_created(&data);
    }
}

#[test]
fn truncated_valid_inputs_never_panic() {
    let auth_key = test_auth_key();
    let auth_key_id = mtproto2::auth_key_id(&auth_key);

    let plain = frame::build_plain_frame(123, b"hello world").unwrap();
    let sealed = frame::seal_frame(
        &auth_key,
        Direction::ClientToServer,
        auth_key_id,
        1,
        0,
        2,
        1,
        b"payload",
    )
    .unwrap();
    let container_body = container::build_container(&[container::ContainerMessage {
        msg_id: 1,
        seq_no: 1,
        body: vec![0x78, 0x56, 0x34, 0x12],
    }])
    .unwrap();
    let rpc_body = rpc::build_rpc_result(7, &[0x78, 0x56, 0x34, 0x12]).unwrap();
    let salt_body = salt::build_new_session_created(1, 2, 3);

    // Каждый обрезанный префикс корректного кадра должен вернуть Err (или Ok
    // для самомладкой длины), но ни при каких условиях не паниковать.
    let all_bodies: &[&[u8]] = &[&plain, &sealed, &container_body, &rpc_body, &salt_body];
    for body in all_bodies {
        for len in 0..body.len() {
            let _ = frame::parse_plain_frame(&body[..len]);
            let _ = frame::open_frame(
                &auth_key,
                Direction::ClientToServer,
                auth_key_id,
                &body[..len],
            );
            let _ = container::parse_container(&body[..len]);
            let _ = rpc::parse_rpc_result(&body[..len]);
            let _ = salt::parse_new_session_created(&body[..len]);
        }
    }
}
