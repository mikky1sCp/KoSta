//! Шифрование сообщений MTProto 2.0: `msg_key`, ключи AES-256-IGE и
//! сборка/разбор зашифрованного конверта.
//!
//! Формат кадра:
//!
//! ```text
//! auth_key_id   : long     (8 байт)
//! msg_key       : int128   (16 байт)
//! encrypted_data: AES-256-IGE(padded_plaintext)   (кратно 16 байтам)
//! ```
//!
//! `padded_plaintext` = `msg_id:long | seq_no:int | body:TL` + случайный
//! padding до кратности 16 (12..1024 байт). Целостность обеспечивается не MAC,
//! а пересчётом `msg_key` от расшифрованного текста: при несовпадении кадр
//! отбрасывается (см. [`open`]).
//!
//! Направление влияет на смещение `x` в KDF: `x = 0` для client → server,
//! `x = 8` для server → client.
//!
//! Формулы соответствуют MTProto 2.0
//! (<https://core.telegram.org/mtproto/description>). В песочнице без сети
//! они не сверялись с оригиналом; при появлении сети — сверить и обновить
//! KAT (`scripts/mtproto2-vectors/gen_vectors.py`).

use rand::RngCore;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::CryptoError;
use crate::ige::{self, AesKey, Iv};

/// Длина `auth_key` в MTProto 2.0: сырой 2048-битный DH-секрет.
pub const AUTH_KEY_LEN: usize = 256;
/// Длина `msg_key`.
pub const MSG_KEY_LEN: usize = 16;
/// Минимальная длина паддинга (байты).
pub const MIN_PADDING: usize = 12;
/// Максимальная длина паддинга (байты).
pub const MAX_PADDING: usize = 1024;

/// `auth_key` MTProto 2.0 — ровно 256 байт big-endian DH-секрета.
pub type AuthKey = [u8; AUTH_KEY_LEN];

/// Кто отправил сообщение. Определяет смещение `x` в KDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Сообщение отправлено клиентом (`x = 0`).
    ClientToServer,
    /// Сообщение отправлено сервером (`x = 8`).
    ServerToClient,
}

impl Direction {
    /// Смещение `x` в формулах MTProto 2.0.
    pub const fn offset(self) -> usize {
        match self {
            Direction::ClientToServer => 0,
            Direction::ServerToClient => 8,
        }
    }
}

/// Результат [`seal`]: `msg_key` и шифротекст (без `auth_key_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedMessage {
    pub msg_key: [u8; MSG_KEY_LEN],
    pub ciphertext: Vec<u8>,
}

/// `auth_key_id` = первые 8 байт `SHA1(auth_key)[12..20]` (LE i64).
pub fn auth_key_id(auth_key: &AuthKey) -> i64 {
    let digest = Sha1::digest(auth_key);
    let mut id_bytes = [0u8; 8];
    id_bytes.copy_from_slice(&digest[12..20]);
    i64::from_le_bytes(id_bytes)
}

/// Вычисляет `msg_key` для padded plaintext.
pub fn msg_key(auth_key: &AuthKey, direction: Direction, plaintext: &[u8]) -> [u8; MSG_KEY_LEN] {
    let x = direction.offset();
    let mut hasher = Sha256::new();
    hasher.update(&auth_key[88 + x..120 + x]);
    hasher.update(plaintext);
    let digest = hasher.finalize();
    let mut key = [0u8; MSG_KEY_LEN];
    key.copy_from_slice(&digest[8..24]);
    key
}

/// Выводит AES-ключ и IV из `auth_key` и `msg_key`.
pub fn derive_aes(
    auth_key: &AuthKey,
    direction: Direction,
    msg_key: &[u8; MSG_KEY_LEN],
) -> (AesKey, Iv) {
    let x = direction.offset();

    let mut hasher_a = Sha256::new();
    hasher_a.update(msg_key);
    hasher_a.update(&auth_key[x..x + 36]);
    let sha_a = hasher_a.finalize();

    let mut hasher_b = Sha256::new();
    hasher_b.update(&auth_key[40 + x..76 + x]);
    hasher_b.update(msg_key);
    let sha_b = hasher_b.finalize();

    let mut key = [0u8; 32];
    let mut iv = [0u8; 32];

    key[0..8].copy_from_slice(&sha_a[0..8]);
    key[8..24].copy_from_slice(&sha_b[8..24]);
    key[24..32].copy_from_slice(&sha_a[24..32]);

    iv[0..8].copy_from_slice(&sha_b[0..8]);
    iv[8..24].copy_from_slice(&sha_a[8..24]);
    iv[24..32].copy_from_slice(&sha_b[24..32]);

    (key, iv)
}

/// Длина случайного паддинга, которая делает `len + pad` кратным 16 и
/// укладывает `pad` в `MIN_PADDING..=MAX_PADDING`.
fn padding_len(len: usize, extra_blocks: usize) -> usize {
    let mut pad = (16 - (len % 16)) % 16;
    if pad < MIN_PADDING {
        pad += 16;
    }
    pad + extra_blocks * 16
}

/// Ограничивает число дополнительных блоков паддинга сверху.
fn max_extra_blocks(base_pad: usize) -> usize {
    (MAX_PADDING.saturating_sub(base_pad)) / 16
}

/// Шифрует `plaintext` (msg_id + seq_no + TL body) и добавляет паддинг.
pub fn seal(
    auth_key: &AuthKey,
    direction: Direction,
    plaintext: &[u8],
) -> Result<SealedMessage, CryptoError> {
    let base_pad = padding_len(plaintext.len(), 0);
    let mut rng = rand::thread_rng();
    let extra = (rng.next_u32() as usize) % (max_extra_blocks(base_pad) + 1);
    let pad = padding_len(plaintext.len(), extra);

    let mut padded = Vec::with_capacity(plaintext.len() + pad);
    padded.extend_from_slice(plaintext);
    let mut padding = vec![0u8; pad];
    rng.fill_bytes(&mut padding);
    padded.extend_from_slice(&padding);

    let msg_key = msg_key(auth_key, direction, &padded);
    let (key, iv) = derive_aes(auth_key, direction, &msg_key);
    let ciphertext = ige::encrypt(&key, &iv, &padded)?;

    Ok(SealedMessage {
        msg_key,
        ciphertext,
    })
}

/// Расшифровывает и проверяет `msg_key`.
///
/// Возвращает padded plaintext: длина TL-тела определяется вызывающим кодом
/// при разборе, а оставшиеся байты — паддинг (см. [`check_padding`]).
pub fn open(
    auth_key: &AuthKey,
    direction: Direction,
    received_msg_key: &[u8; MSG_KEY_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(ige::BLOCK) {
        return Err(CryptoError::Custom(format!(
            "invalid MTProto 2.0 ciphertext length: {}",
            ciphertext.len()
        )));
    }

    let (key, iv) = derive_aes(auth_key, direction, received_msg_key);
    let plaintext = ige::decrypt(&key, &iv, ciphertext)?;

    let expected = msg_key(auth_key, direction, &plaintext);
    if expected.ct_eq(received_msg_key).unwrap_u8() != 1 {
        return Err(CryptoError::Custom("MTProto 2.0 msg_key mismatch".into()));
    }

    Ok(plaintext)
}

/// Проверяет, что после разбора тела осталось допустимое количество
/// паддинг-байт (12..1024). `total` — длина расшифрованного plaintext,
/// `parsed` — сколько байт занял `msg_id | seq_no | body`.
pub fn check_padding(total: usize, parsed: usize) -> Result<usize, CryptoError> {
    if parsed > total {
        return Err(CryptoError::Custom(format!(
            "parsed length {} exceeds plaintext length {}",
            parsed, total
        )));
    }
    let leftover = total - parsed;
    if !(MIN_PADDING..=MAX_PADDING).contains(&leftover) {
        return Err(CryptoError::Custom(format!(
            "invalid MTProto 2.0 padding length: {}",
            leftover
        )));
    }
    Ok(leftover)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KAT сгенерирован независимой Python-реализацией:
    /// `scripts/mtproto2-vectors/gen_vectors.py`.
    const MSG_KEY_CLIENT_HEX: &str = "09186cc63d9fbae52d85b688c55b8676";
    const AES_KEY_CLIENT_HEX: &str =
        "1e58ba5df3f4117f6bdf9677116d4d9cf7fc4cbcc398f9cfab92d6020eecf3f8";
    const AES_IV_CLIENT_HEX: &str =
        "18e57b49912129233430866186d5412769f3980ade083b904a0590d49f4a30d0";
    const MSG_KEY_SERVER_HEX: &str = "5a796c4fdc8d56c6d1de120db2e2ff5b";
    const AES_KEY_SERVER_HEX: &str =
        "db9a6d6f332a65d3b31a3e38e2f36556b3d9a33f7cb9b646ef4f4812042e6aff";
    const AES_IV_SERVER_HEX: &str =
        "63cf5c63285ab6e0ebeae144b0664ce8a04df1984a99f5f8cb52c6b6928078a5";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    fn kat_auth_key() -> AuthKey {
        let mut key = [0u8; AUTH_KEY_LEN];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = ((i * 7 + 11) % 256) as u8;
        }
        key
    }

    fn kat_plaintext() -> Vec<u8> {
        (0x30u8..0x30 + 48).collect()
    }

    #[test]
    fn kdf_known_answer() {
        let auth_key = kat_auth_key();
        let plaintext = kat_plaintext();

        let client_key = msg_key(&auth_key, Direction::ClientToServer, &plaintext);
        assert_eq!(client_key.to_vec(), hex(MSG_KEY_CLIENT_HEX));
        let (client_aes_key, client_iv) =
            derive_aes(&auth_key, Direction::ClientToServer, &client_key);
        assert_eq!(client_aes_key.to_vec(), hex(AES_KEY_CLIENT_HEX));
        assert_eq!(client_iv.to_vec(), hex(AES_IV_CLIENT_HEX));

        let server_key = msg_key(&auth_key, Direction::ServerToClient, &plaintext);
        assert_eq!(server_key.to_vec(), hex(MSG_KEY_SERVER_HEX));
        let (server_aes_key, server_iv) =
            derive_aes(&auth_key, Direction::ServerToClient, &server_key);
        assert_eq!(server_aes_key.to_vec(), hex(AES_KEY_SERVER_HEX));
        assert_eq!(server_iv.to_vec(), hex(AES_IV_SERVER_HEX));
    }

    #[test]
    fn seal_open_roundtrip_in_both_directions() {
        let auth_key = kat_auth_key();
        let plaintext = b"msg_id + seq_no + TL body";
        for direction in [Direction::ClientToServer, Direction::ServerToClient] {
            let sealed = seal(&auth_key, direction, plaintext).unwrap();
            assert!(sealed.ciphertext.len().is_multiple_of(ige::BLOCK));
            let opened = open(&auth_key, direction, &sealed.msg_key, &sealed.ciphertext).unwrap();
            assert_eq!(&opened[..plaintext.len()], plaintext);
            let padding = check_padding(opened.len(), plaintext.len()).unwrap();
            assert!((MIN_PADDING..=MAX_PADDING).contains(&padding));
        }
    }

    #[test]
    fn wrong_direction_is_rejected() {
        let auth_key = kat_auth_key();
        let sealed = seal(&auth_key, Direction::ClientToServer, b"payload").unwrap();
        let result = open(
            &auth_key,
            Direction::ServerToClient,
            &sealed.msg_key,
            &sealed.ciphertext,
        );
        assert!(
            result.is_err(),
            "server-direction key must not decrypt client frame"
        );
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let auth_key = kat_auth_key();
        let sealed = seal(&auth_key, Direction::ClientToServer, b"payload").unwrap();
        let mut ciphertext = sealed.ciphertext.clone();
        ciphertext[0] ^= 0x01;
        assert!(open(
            &auth_key,
            Direction::ClientToServer,
            &sealed.msg_key,
            &ciphertext
        )
        .is_err());
    }

    #[test]
    fn tampered_msg_key_is_rejected() {
        let auth_key = kat_auth_key();
        let sealed = seal(&auth_key, Direction::ClientToServer, b"payload").unwrap();
        let mut msg_key = sealed.msg_key;
        msg_key[0] ^= 0xff;
        assert!(open(
            &auth_key,
            Direction::ClientToServer,
            &msg_key,
            &sealed.ciphertext
        )
        .is_err());
    }

    #[test]
    fn wrong_auth_key_is_rejected() {
        let auth_key = kat_auth_key();
        let sealed = seal(&auth_key, Direction::ClientToServer, b"payload").unwrap();
        let mut other_key = auth_key;
        other_key[0] ^= 0x01;
        assert!(open(
            &other_key,
            Direction::ClientToServer,
            &sealed.msg_key,
            &sealed.ciphertext
        )
        .is_err());
    }

    #[test]
    fn invalid_ciphertext_length_is_rejected() {
        let auth_key = kat_auth_key();
        let msg_key = [0u8; MSG_KEY_LEN];
        assert!(open(&auth_key, Direction::ClientToServer, &msg_key, &[]).is_err());
        assert!(open(&auth_key, Direction::ClientToServer, &msg_key, &[1, 2, 3]).is_err());
        assert!(open(&auth_key, Direction::ClientToServer, &msg_key, &[0u8; 17]).is_err());
    }

    #[test]
    fn padding_lengths_are_within_limits() {
        let auth_key = kat_auth_key();
        for len in [0usize, 1, 15, 16, 17, 1000, 4096] {
            let plaintext = vec![0xABu8; len];
            let sealed = seal(&auth_key, Direction::ClientToServer, &plaintext).unwrap();
            let opened = open(
                &auth_key,
                Direction::ClientToServer,
                &sealed.msg_key,
                &sealed.ciphertext,
            )
            .unwrap();
            let pad = check_padding(opened.len(), len).unwrap();
            assert!(
                (MIN_PADDING..=MAX_PADDING).contains(&pad),
                "len={} pad={}",
                len,
                pad
            );
        }
    }

    #[test]
    fn check_padding_rejects_invalid_leftover() {
        assert!(check_padding(100, 99).is_err(), "leftover 1 is too small");
        assert!(check_padding(2000, 1).is_err(), "leftover 1999 is too big");
        assert!(check_padding(10, 11).is_err(), "parsed > total");
        assert_eq!(check_padding(100, 88).unwrap(), 12);
    }

    #[test]
    fn auth_key_id_is_stable() {
        let auth_key = kat_auth_key();
        let id = auth_key_id(&auth_key);
        assert_eq!(id, auth_key_id(&kat_auth_key()));
        let mut other = auth_key;
        other[255] ^= 0x01;
        assert_ne!(id, auth_key_id(&other));
    }

    /// Детерминированный xorshift (без внешних rand-фич), чтобы тест был
    /// воспроизводим в любом окружении.
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

    #[test]
    fn randomized_roundtrip_across_many_plaintext_lengths() {
        let auth_key = kat_auth_key();
        let mut rng = Xorshift(0x5EED_5EED_0C0F_0FEE);
        for direction in [Direction::ClientToServer, Direction::ServerToClient] {
            for _ in 0..400 {
                let len = (rng.next() % 3200) as usize;
                let plaintext = rng.bytes(len);
                let sealed = seal(&auth_key, direction, &plaintext).unwrap();
                assert!(sealed.ciphertext.len().is_multiple_of(ige::BLOCK));
                let opened = open(&auth_key, direction, &sealed.msg_key, &sealed.ciphertext).unwrap();
                assert_eq!(&opened[..len], &plaintext[..], "len={len}");
                let pad = check_padding(opened.len(), len).unwrap();
                assert!((MIN_PADDING..=MAX_PADDING).contains(&pad), "len={len} pad={pad}");
            }
        }
    }

    #[test]
    fn every_random_ciphertext_tamper_is_rejected() {
        let auth_key = kat_auth_key();
        let plaintext = vec![0xABu8; 64];
        let sealed = seal(&auth_key, Direction::ClientToServer, &plaintext).unwrap();
        let mut rng = Xorshift(0xFACE_BEEF);
        for _ in 0..300 {
            let pos = (rng.next() % sealed.ciphertext.len() as u64) as usize;
            let mut tampered = sealed.ciphertext.clone();
            tampered[pos] ^= 0b1010_1010;
            assert!(
                open(&auth_key, Direction::ClientToServer, &sealed.msg_key, &tampered).is_err(),
                "изменение байта {pos} не должно расшифровываться"
            );
        }
    }
}
