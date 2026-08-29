pub mod error;
pub mod keys;
pub mod kdf;
pub mod ige;
pub mod padding;
pub mod dh;
pub mod encrypted_inner;
pub mod ctr;
pub mod session_crypto;
pub mod gcm;  // <-- добавить

pub use error::CryptoError;
pub use keys::AuthKey;

#[cfg(test)]
mod crypto_tests {
    use super::*;
    use crate::session_crypto::SessionCrypto;

    #[test]
    fn test_aes_ctr() {
        let key = [0u8; 32];
        let nonce = [0u8; 16];
        let plain = b"Hello, World!";
        let cipher = ctr::encrypt(&key, &nonce, plain).unwrap();
        let dec = ctr::decrypt(&key, &nonce, &cipher).unwrap();
        assert_eq!(plain, &dec[..]);
    }

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

    #[test]
    fn test_session_crypto() {
        let shared = [1u8; 32];
        let client_nonce = [2u8; 16];
        let server_nonce = [3u8; 16];
        let mut crypto = SessionCrypto::new(&shared, &client_nonce, &server_nonce);
        let plain = b"Test message";
        let (nonce, cipher, tag) = crypto.encrypt_outgoing(plain).unwrap();
        let dec = crypto.decrypt_incoming(&nonce, &cipher, &tag).unwrap();
        assert_eq!(plain, &dec[..]);
    }
}