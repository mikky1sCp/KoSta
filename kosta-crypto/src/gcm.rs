use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, KeyInit};
use crate::error::CryptoError;

pub fn encrypt_gcm(key: &[u8; 32], nonce: &[u8; 12], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let ciphertext = cipher.encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::Custom(format!("GCM encryption failed: {}", e)))?;
    Ok(ciphertext) // содержит тег (16 байт) в конце
}

pub fn decrypt_gcm(key: &[u8; 32], nonce: &[u8; 12], ciphertext_with_tag: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let plaintext = cipher.decrypt(nonce, ciphertext_with_tag)
        .map_err(|e| CryptoError::Custom(format!("GCM decryption failed: {}", e)))?;
    Ok(plaintext)
}