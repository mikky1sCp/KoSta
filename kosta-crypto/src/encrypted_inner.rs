use crate::error::CryptoError;
use crate::gcm;

/// Временный ключ для шифрования `PqInnerData` (шаг `req_DH_params`).
pub fn tmp_key_from_nonce_bytes(nonce: &[u8; 16], server_nonce: &[u8; 16]) -> ([u8; 32], [u8; 12]) {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(nonce);
    hasher.update(server_nonce);
    let hash = hasher.finalize();
    let mut aes_key = [0u8; 32];
    aes_key.copy_from_slice(&hash[..32]);

    let mut hasher2 = Sha256::new();
    hasher2.update(server_nonce);
    hasher2.update(nonce);
    let hash2 = hasher2.finalize();
    let mut nonce_gcm = [0u8; 12];
    nonce_gcm.copy_from_slice(&hash2[..12]);

    (aes_key, nonce_gcm)
}

/// Временный ключ для шифрования `ServerDHInnerData`/`ClientDHInnerData`.
pub fn tmp_key_bytes(new_nonce: &[u8; 32], server_nonce: &[u8; 16]) -> ([u8; 32], [u8; 12]) {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(new_nonce);
    hasher.update(server_nonce);
    let hash = hasher.finalize();
    let mut aes_key = [0u8; 32];
    aes_key.copy_from_slice(&hash[..32]);

    let mut hasher2 = Sha256::new();
    hasher2.update(server_nonce);
    hasher2.update(new_nonce);
    let hash2 = hasher2.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&hash2[..12]);

    (aes_key, nonce)
}

pub fn encrypt_inner(
    key: &[u8; 32],
    nonce: &[u8; 12],
    data: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    gcm::encrypt_gcm(key, nonce, data)
}

pub fn decrypt_inner(
    key: &[u8; 32],
    nonce: &[u8; 12],
    data: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    gcm::decrypt_gcm(key, nonce, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_nonces_give_different_keys() {
        let a = tmp_key_from_nonce_bytes(&[1u8; 16], &[2u8; 16]);
        let b = tmp_key_from_nonce_bytes(&[1u8; 16], &[3u8; 16]);
        assert_ne!(a.0, b.0);
        assert_ne!(a.1, b.1);
    }
}
