use aes::Aes256;
use ctr::Ctr128BE;
use cipher::{KeyIvInit, StreamCipher};
use cipher::generic_array::GenericArray;
use crate::error::CryptoError;

pub type Aes256Ctr = Ctr128BE<Aes256>;

pub fn encrypt(key: &[u8; 32], nonce: &[u8; 16], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key_arr = GenericArray::from_slice(key);
    let nonce_arr = GenericArray::from_slice(nonce);
    let mut cipher = Aes256Ctr::new(key_arr, nonce_arr);
    let mut ciphertext = plaintext.to_vec();
    cipher.apply_keystream(&mut ciphertext);
    Ok(ciphertext)
}

pub fn decrypt(key: &[u8; 32], nonce: &[u8; 16], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    encrypt(key, nonce, ciphertext)
}