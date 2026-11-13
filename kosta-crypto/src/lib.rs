//! Криптографические примитивы KoSta.
//!
//! * [`dh`] / [`dh_checks`] — Диффи-Хеллман и проверки параметров;
//! * [`encrypted_inner`] — AES-GCM для внутренних данных хендшейка;
//! * [`ige`] — AES-256-IGE;
//! * [`mtproto2`] — `msg_key`, KDF, sealing/opening сообщений MTProto 2.0;
//! * [`handshake`] — `new_nonce_hash1/3`;
//! * [`padding`] — случайный паддинг до кратности 16;
//! * [`gcm`] — AES-256-GCM (секреты в БД, inner-данные).

pub mod dh;
pub mod dh_checks;
pub mod encrypted_inner;
pub mod error;
pub mod gcm;
pub mod handshake;
pub mod ige;
pub mod mtproto2;
pub mod padding;

pub use error::CryptoError;

#[cfg(test)]
mod crypto_tests {
    use super::*;

    #[test]
    fn test_gcm() {
        let key = [0u8; 32];
        let nonce = [0u8; 12];
        let plain = b"Secret data";
        let cipher = gcm::encrypt_gcm(&key, &nonce, plain).unwrap();
        let dec = gcm::decrypt_gcm(&key, &nonce, &cipher).unwrap();
        assert_eq!(plain, &dec[..]);
    }

    #[test]
    fn test_dh() {
        let p = num_bigint::BigUint::from(23u32);
        let g = num_bigint::BigUint::from(5u32);
        let a = dh::generate_private_key() % &p;
        let b = dh::generate_private_key() % &p;
        let pub_a = dh::compute_public_key(&g, &a, &p);
        let pub_b = dh::compute_public_key(&g, &b, &p);
        let secret_a = dh::compute_shared_secret(&pub_b, &a, &p);
        let secret_b = dh::compute_shared_secret(&pub_a, &b, &p);
        assert_eq!(secret_a, secret_b);
    }
}
