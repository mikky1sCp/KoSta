use aes::Aes256;
use cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use crate::error::CryptoError;

type Block = [u8; 16];

pub fn ige_encrypt(key: &[u8; 32], iv: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256::new_from_slice(key).map_err(CryptoError::Aes)?;
    let mut c: Block = iv[0..16].try_into().unwrap();
    let mut m: Block = iv[16..32].try_into().unwrap();
    let mut encrypted = Vec::with_capacity(plaintext.len());
    for chunk in plaintext.chunks(16) {
        let block: Block = chunk.try_into().map_err(|_| CryptoError::Custom("Input not aligned to 16 bytes".into()))?;
        let mut x = block;
        for (x_byte, c_byte) in x.iter_mut().zip(c.iter()) {
            *x_byte ^= c_byte;
        }
        cipher.encrypt_block((&mut x).into());
        for (x_byte, m_byte) in x.iter_mut().zip(m.iter()) {
            *x_byte ^= m_byte;
        }
        encrypted.extend_from_slice(&x);
        m = block;
        c = x;
    }
    Ok(encrypted)
}

pub fn ige_decrypt(key: &[u8; 32], iv: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256::new_from_slice(key).map_err(CryptoError::Aes)?;
    let mut c: Block = iv[0..16].try_into().unwrap();
    let mut m: Block = iv[16..32].try_into().unwrap();
    let mut decrypted = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks(16) {
        let block: Block = chunk.try_into().map_err(|_| CryptoError::Custom("Input not aligned to 16 bytes".into()))?;
        let mut temp = block;
        for (t, m_byte) in temp.iter_mut().zip(m.iter()) {
            *t ^= m_byte;
        }
        cipher.decrypt_block((&mut temp).into());
        let mut plain = temp;
        for (p, c_byte) in plain.iter_mut().zip(c.iter()) {
            *p ^= c_byte;
        }
        decrypted.extend_from_slice(&plain);
        m = plain;
        c = block;
    }
    Ok(decrypted)
}