use sha2::{Sha256, Digest};
use num_bigint::BigUint;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthKey(pub [u8; 256]);

impl AuthKey {
    pub fn msg_key(&self, plaintext: &[u8]) -> [u8; 16] {
        let mut hasher = Sha256::new();
        hasher.update(&self.0[0..32]);
        hasher.update(plaintext);
        let hash = hasher.finalize();
        hash[8..24].try_into().unwrap()
    }

    pub fn kdf(&self, msg_key: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
        let auth_key = &self.0;

        let mut hasher_a = Sha256::new();
        hasher_a.update(msg_key);
        hasher_a.update(&auth_key[0..36]);
        let sha256_a = hasher_a.finalize();

        let mut hasher_b = Sha256::new();
        hasher_b.update(&auth_key[40..56]);
        hasher_b.update(msg_key);
        hasher_b.update(&auth_key[0..16]);
        let sha256_b = hasher_b.finalize();

        let mut aes_key = [0u8; 32];
        let mut aes_iv = [0u8; 32];

        aes_key[0..8].copy_from_slice(&sha256_a[0..8]);
        aes_key[8..24].copy_from_slice(&sha256_b[8..24]);
        aes_key[24..32].copy_from_slice(&sha256_a[24..32]);

        aes_iv[0..8].copy_from_slice(&sha256_b[0..8]);
        aes_iv[8..24].copy_from_slice(&sha256_a[8..24]);
        aes_iv[24..32].copy_from_slice(&sha256_b[24..32]);

        (aes_key, aes_iv)
    }

    /// Генерирует auth_key из общего секрета DH, nonce и server_nonce.
    pub fn from_shared_secret(
        secret: &BigUint,
        nonce: &[u8; 16],
        server_nonce: &[u8; 16],
    ) -> Self {
        let secret_bytes = secret.to_bytes_le();
        let mut auth_key = [0u8; 256];
        let mut hasher = Sha256::new();
        hasher.update(&secret_bytes);
        hasher.update(nonce);
        hasher.update(server_nonce);
        let hash = hasher.finalize();
        auth_key[0..32].copy_from_slice(&hash);

        // Расширяем до 256 байт, хешируя предыдущий блок с секретом
        for i in 1..8 {
            let mut h = Sha256::new();
            h.update(&auth_key[(i-1)*32..i*32]);
            h.update(&secret_bytes);
            let next = h.finalize();
            auth_key[i*32..(i+1)*32].copy_from_slice(&next);
        }
        AuthKey(auth_key)
    }
}