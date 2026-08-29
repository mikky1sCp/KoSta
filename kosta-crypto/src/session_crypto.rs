// kosta-crypto/src/session_crypto.rs
use hkdf::Hkdf;
use sha2::Sha256;
use hmac::{Hmac, Mac};
use rand::{RngCore, thread_rng};
use subtle::ConstantTimeEq;
use crate::error::CryptoError;

// Типы для ключей
pub type AesKey = [u8; 32];
pub type MacKey = [u8; 32];
pub type Nonce = [u8; 16];
pub type Tag = [u8; 16];

/// Выводит 4 ключа из общего секрета и двух nonce.
/// Возвращает (client_write_key, client_mac_key, server_write_key, server_mac_key)
pub fn derive_keys(
    shared_secret: &[u8],
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
) -> (AesKey, MacKey, AesKey, MacKey) {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(client_nonce);
    hasher.update(server_nonce);
    let salt = hasher.finalize();

    let hk = Hkdf::<Sha256>::new(Some(&salt), shared_secret);

    let mut okm = [0u8; 128]; // 4 * 32
    hk.expand(b"kosta-keys", &mut okm)
        .expect("HKDF expand should not fail");

    let mut client_write = [0u8; 32];
    let mut client_mac = [0u8; 32];
    let mut server_write = [0u8; 32];
    let mut server_mac = [0u8; 32];

    client_write.copy_from_slice(&okm[0..32]);
    client_mac.copy_from_slice(&okm[32..64]);
    server_write.copy_from_slice(&okm[64..96]);
    server_mac.copy_from_slice(&okm[96..128]);

    (client_write, client_mac, server_write, server_mac)
}

/// Генерирует уникальный nonce (16 байт) для каждого сообщения.
/// Первые 4 байта – счётчик (увеличивается), остальные 12 – случайные.
pub fn generate_nonce(counter: u32) -> Nonce {
    let mut nonce = [0u8; 16];
    nonce[0..4].copy_from_slice(&counter.to_be_bytes());
    thread_rng().fill_bytes(&mut nonce[4..16]);
    nonce
}

/// Шифрует и аутентифицирует сообщение.
/// Возвращает (ciphertext, tag)
pub fn encrypt_and_tag(
    aes_key: &AesKey,
    mac_key: &MacKey,
    nonce: &Nonce,
    plaintext: &[u8],
) -> Result<(Vec<u8>, Tag), CryptoError> {
    let ciphertext = crate::ctr::encrypt(aes_key, nonce, plaintext)?;

    let mut mac = Hmac::<Sha256>::new_from_slice(mac_key)
        .map_err(|_| CryptoError::Custom("Invalid MAC key length".into()))?;
    mac.update(nonce);
    mac.update(&ciphertext);
    let result = mac.finalize();
    let full_tag = result.into_bytes();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&full_tag[..16]);

    Ok((ciphertext, tag))
}

/// Расшифровывает и проверяет аутентификацию.
pub fn decrypt_and_verify(
    aes_key: &AesKey,
    mac_key: &MacKey,
    nonce: &Nonce,
    ciphertext: &[u8],
    tag: &Tag,
) -> Result<Vec<u8>, CryptoError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(mac_key)
        .map_err(|_| CryptoError::Custom("Invalid MAC key length".into()))?;
    mac.update(nonce);
    mac.update(ciphertext);
    let result = mac.finalize();
    let full_tag = result.into_bytes();
    let expected_tag = &full_tag[..16];

    if expected_tag.ct_eq(tag).unwrap_u8() != 1 {
        return Err(CryptoError::Custom("MAC verification failed".into()));
    }

    crate::ctr::decrypt(aes_key, nonce, ciphertext)
}

/// Структура для удобной работы с сессионной криптографией
#[derive(Clone)]
pub struct SessionCrypto {
    pub client_write_key: AesKey,
    pub client_mac_key: MacKey,
    pub server_write_key: AesKey,
    pub server_mac_key: MacKey,
    pub send_counter: u32,
    pub recv_counter: u32,
}

impl SessionCrypto {
    pub fn new(
        shared_secret: &[u8],
        client_nonce: &[u8; 16],
        server_nonce: &[u8; 16],
    ) -> Self {
        let (cw, cm, sw, sm) = derive_keys(shared_secret, client_nonce, server_nonce);
        SessionCrypto {
            client_write_key: cw,
            client_mac_key: cm,
            server_write_key: sw,
            server_mac_key: sm,
            send_counter: 0,
            recv_counter: 0,
        }
    }

    /// Шифрует сообщение для отправки (использует client ключи)
    pub fn encrypt_outgoing(&mut self, plaintext: &[u8]) -> Result<(Nonce, Vec<u8>, Tag), CryptoError> {
        let nonce = generate_nonce(self.send_counter);
        self.send_counter += 1;
        let (ciphertext, tag) = encrypt_and_tag(
            &self.client_write_key,
            &self.client_mac_key,
            &nonce,
            plaintext,
        )?;
        Ok((nonce, ciphertext, tag))
    }

    /// Расшифровывает входящее сообщение (использует server ключи)
    pub fn decrypt_incoming(&mut self, nonce: &Nonce, ciphertext: &[u8], tag: &Tag) -> Result<Vec<u8>, CryptoError> {
        let plaintext = decrypt_and_verify(
            &self.server_write_key,
            &self.server_mac_key,
            nonce,
            ciphertext,
            tag,
        )?;
        self.recv_counter += 1;
        Ok(plaintext)
    }

    // ----- НОВЫЕ МЕТОДЫ ДЛЯ СЕРВЕРА -----

    /// Шифрует сообщение для отправки от сервера (использует server ключи)
    pub fn encrypt_outgoing_server(&mut self, plaintext: &[u8]) -> Result<(Nonce, Vec<u8>, Tag), CryptoError> {
        let nonce = generate_nonce(self.send_counter);
        self.send_counter += 1;
        let (ciphertext, tag) = encrypt_and_tag(
            &self.server_write_key,
            &self.server_mac_key,
            &nonce,
            plaintext,
        )?;
        Ok((nonce, ciphertext, tag))
    }

    /// Расшифровывает входящее сообщение для сервера (использует client ключи)
    pub fn decrypt_incoming_server(&mut self, nonce: &Nonce, ciphertext: &[u8], tag: &Tag) -> Result<Vec<u8>, CryptoError> {
        let plaintext = decrypt_and_verify(
            &self.client_write_key,
            &self.client_mac_key,
            nonce,
            ciphertext,
            tag,
        )?;
        self.recv_counter += 1;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn test_encrypt_decrypt() {
        let mut rng = rand::thread_rng();
        let mut shared = [0u8; 32];
        rng.fill_bytes(&mut shared);
        let client_nonce = [1u8; 16];
        let server_nonce = [2u8; 16];

        let (client_write_key, client_mac_key, _server_write_key, _server_mac_key) =
            derive_keys(&shared, &client_nonce, &server_nonce);

        let plaintext = b"Hello, secure protocol!";
        let nonce = generate_nonce(0);
        let (ciphertext, tag) = encrypt_and_tag(&client_write_key, &client_mac_key, &nonce, plaintext).unwrap();
        let decrypted = decrypt_and_verify(&client_write_key, &client_mac_key, &nonce, &ciphertext, &tag).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_invalid_tag() {
        let mut rng = rand::thread_rng();
        let mut shared = [0u8; 32];
        rng.fill_bytes(&mut shared);
        let client_nonce = [1u8; 16];
        let server_nonce = [2u8; 16];

        let (client_write_key, client_mac_key, _server_write_key, _server_mac_key) =
            derive_keys(&shared, &client_nonce, &server_nonce);

        let plaintext = b"test";
        let nonce = generate_nonce(0);
        let (ciphertext, mut tag) = encrypt_and_tag(&client_write_key, &client_mac_key, &nonce, plaintext).unwrap();
        tag[0] ^= 0xff; // corrupt tag

        let result = decrypt_and_verify(&client_write_key, &client_mac_key, &nonce, &ciphertext, &tag);
        assert!(result.is_err());
    }
}