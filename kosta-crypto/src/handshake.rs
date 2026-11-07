//! Хеши хендшейка MTProto (`new_nonce_hash1` / `new_nonce_hash3`).
//!
//! В MTProto значения-подтверждения считаются как
//! `SHA1(new_nonce ‖ число ‖ server_nonce)[4..20]`, где число — `1` для
//! `dh_gen_ok` и `3` для `dh_gen_fail`. Раньше KoSta использовал
//! `SHA1(shared_secret)[0..16]`, что не соответствует спеке.

use sha1::{Digest, Sha1};

/// `new_nonce_hash` для номера `number` (1 или 3).
pub fn new_nonce_hash(new_nonce: &[u8; 32], server_nonce: &[u8; 16], number: u8) -> [u8; 16] {
    let mut hasher = Sha1::new();
    hasher.update(new_nonce);
    hasher.update([number]);
    hasher.update(server_nonce);
    let digest = hasher.finalize();
    let mut result = [0u8; 16];
    result.copy_from_slice(&digest[4..20]);
    result
}

/// `new_nonce_hash1` (`dh_gen_ok`).
pub fn new_nonce_hash1(new_nonce: &[u8; 32], server_nonce: &[u8; 16]) -> [u8; 16] {
    new_nonce_hash(new_nonce, server_nonce, 1)
}

/// `new_nonce_hash3` (`dh_gen_fail`).
pub fn new_nonce_hash3(new_nonce: &[u8; 32], server_nonce: &[u8; 16]) -> [u8; 16] {
    new_nonce_hash(new_nonce, server_nonce, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_deterministic_and_distinct() {
        let new_nonce = [0xABu8; 32];
        let server_nonce = [0xCDu8; 16];

        let ok = new_nonce_hash1(&new_nonce, &server_nonce);
        assert_eq!(ok, new_nonce_hash1(&new_nonce, &server_nonce));
        let fail = new_nonce_hash3(&new_nonce, &server_nonce);
        assert_ne!(ok, fail);
        assert_ne!(ok, [0u8; 16]);
    }

    #[test]
    fn changing_nonce_changes_hash() {
        let server_nonce = [0x11u8; 16];
        let a = new_nonce_hash1(&[1u8; 32], &server_nonce);
        let b = new_nonce_hash1(&[2u8; 32], &server_nonce);
        assert_ne!(a, b);
    }
}
