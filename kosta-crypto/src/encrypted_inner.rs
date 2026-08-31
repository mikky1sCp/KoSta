use crate::error::CryptoError;
use kosta_core::tl::types::{Int128, Int256};
use crate::gcm;

pub fn tmp_key(new_nonce: &Int256, server_nonce: &Int128) -> ([u8; 32], [u8; 12]) {
    use sha2::{Sha256, Digest};

    let mut hasher = Sha256::new();
    hasher.update(&new_nonce.0);
    hasher.update(&server_nonce.0);
    let hash = hasher.finalize();
    let mut aes_key = [0u8; 32];
    aes_key.copy_from_slice(&hash[..32]);

    let mut hasher2 = Sha256::new();
    hasher2.update(&server_nonce.0);
    hasher2.update(&new_nonce.0);
    let hash2 = hasher2.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&hash2[..12]);

    (aes_key, nonce)
}

// Новая функция для временного ключа из двух Int128 (nonce и server_nonce)
pub fn tmp_key_from_nonce(nonce: &Int128, server_nonce: &Int128) -> ([u8; 32], [u8; 12]) {
    use sha2::{Sha256, Digest};

    let mut hasher = Sha256::new();
    hasher.update(&nonce.0);
    hasher.update(&server_nonce.0);
    let hash = hasher.finalize();
    let mut aes_key = [0u8; 32];
    aes_key.copy_from_slice(&hash[..32]);

    let mut hasher2 = Sha256::new();
    hasher2.update(&server_nonce.0);
    hasher2.update(&nonce.0);
    let hash2 = hasher2.finalize();
    let mut nonce_gcm = [0u8; 12];
    nonce_gcm.copy_from_slice(&hash2[..12]);

    (aes_key, nonce_gcm)
}

pub fn encrypt_inner(key: &[u8; 32], nonce: &[u8; 12], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    gcm::encrypt_gcm(key, nonce, data)
}

pub fn decrypt_inner(key: &[u8; 32], nonce: &[u8; 12], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    gcm::decrypt_gcm(key, nonce, data)
}