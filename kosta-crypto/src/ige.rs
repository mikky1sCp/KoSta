//! AES-256-IGE (Infinite Garble Extension) — режим блочного шифрования,
//! который использует MTProto 2.0 для тела сообщений.
//!
//! IGE — нестандартный режим: каждый блок «зацепляется» сразу за двумя
//! предыдущими блоками (открытый текст и шифротекст), а IV состоит из двух
//! 16-байтовых половин. Реализация повторяет семантику OpenSSL `AES_ige_encrypt`,
//! на которую опирается MTProto:
//!
//! ```text
//! C_i = AES_enc(P_i XOR C_{i-1}) XOR P_{i-1}
//! P_i = AES_dec(C_i XOR P_{i-1}) XOR C_{i-1}
//! ```
//!
//! где `C_{-1}` — первая половина IV, `P_{-1}` — вторая.

use aes::Aes256;
use cipher::generic_array::GenericArray;
use cipher::{BlockDecrypt, BlockEncrypt, KeyInit};

use crate::error::CryptoError;

/// Размер блока AES.
pub const BLOCK: usize = 16;
/// Длина IV в IGE: две половины по 16 байт.
pub const IV_LEN: usize = 32;

/// Ключ AES-256.
pub type AesKey = [u8; 32];
/// IV для IGE (две половины).
pub type Iv = [u8; IV_LEN];

fn validate(key: &AesKey, iv: &Iv, data: &[u8]) -> Result<(), CryptoError> {
    let _ = (key, iv);
    if !data.len().is_multiple_of(BLOCK) {
        return Err(CryptoError::Custom(format!(
            "AES-IGE input length {} is not a multiple of {}",
            data.len(),
            BLOCK
        )));
    }
    Ok(())
}

/// Шифрует `data` в режиме AES-256-IGE.
///
/// `data.len()` обязан быть кратен 16; padding не добавляется — это забота
/// вызывающего кода (в MTProto сообщение паддится перед шифрованием).
pub fn encrypt(key: &AesKey, iv: &Iv, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    validate(key, iv, data)?;
    let cipher = Aes256::new_from_slice(key)
        .map_err(|_| CryptoError::Custom("invalid AES-256 key length".into()))?;

    let mut out = Vec::with_capacity(data.len());
    let mut prev_c = [0u8; BLOCK];
    prev_c.copy_from_slice(&iv[0..BLOCK]);
    let mut prev_p = [0u8; BLOCK];
    prev_p.copy_from_slice(&iv[BLOCK..IV_LEN]);

    for chunk in data.as_chunks::<BLOCK>().0 {
        let mut block = GenericArray::clone_from_slice(chunk);
        for i in 0..BLOCK {
            block[i] ^= prev_c[i];
        }
        cipher.encrypt_block(&mut block);
        for i in 0..BLOCK {
            block[i] ^= prev_p[i];
        }
        out.extend_from_slice(&block);

        prev_c.copy_from_slice(&block);
        prev_p.copy_from_slice(chunk);
    }
    Ok(out)
}

/// Расшифровывает `data` в режиме AES-256-IGE.
///
/// Проверки целостности здесь нет: MTProto 2.0 аутентифицирует сообщение
/// через `msg_key` (см. [`crate::mtproto2::open`]).
pub fn decrypt(key: &AesKey, iv: &Iv, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    validate(key, iv, data)?;
    let cipher = Aes256::new_from_slice(key)
        .map_err(|_| CryptoError::Custom("invalid AES-256 key length".into()))?;

    let mut out = Vec::with_capacity(data.len());
    let mut prev_c = [0u8; BLOCK];
    prev_c.copy_from_slice(&iv[0..BLOCK]);
    let mut prev_p = [0u8; BLOCK];
    prev_p.copy_from_slice(&iv[BLOCK..IV_LEN]);

    for chunk in data.as_chunks::<BLOCK>().0 {
        let mut block = GenericArray::clone_from_slice(chunk);
        for i in 0..BLOCK {
            block[i] ^= prev_p[i];
        }
        cipher.decrypt_block(&mut block);
        for i in 0..BLOCK {
            block[i] ^= prev_c[i];
        }
        out.extend_from_slice(&block);

        prev_c.copy_from_slice(chunk);
        prev_p.copy_from_slice(&block);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KAT сгенерирован независимой Python-реализацией:
    /// `scripts/mtproto2-vectors/gen_vectors.py` (pycryptodome).
    const IGE_KAT_KEY_HEX: &str =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const IGE_KAT_IV_HEX: &str = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf";
    const IGE_KAT_PLAINTEXT_HEX: &str = concat!(
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
    );
    const IGE_KAT_CIPHERTEXT_HEX: &str = concat!(
        "92fd95470e823d906b6f32316050dafd3bcecb7858bd74dfd74ed6a8a040be8c",
        "d488fbab83a0bee7c63a24f0f6043bf99ef26d38200662aac248a3579d6884c4",
    );

    fn hex(s: &str) -> Vec<u8> {
        assert!(s.len().is_multiple_of(2), "odd hex length");
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    #[test]
    fn ige_known_answer() {
        let key: AesKey = hex(IGE_KAT_KEY_HEX).try_into().unwrap();
        let iv: Iv = hex(IGE_KAT_IV_HEX).try_into().unwrap();
        let plaintext = hex(IGE_KAT_PLAINTEXT_HEX);
        let expected = hex(IGE_KAT_CIPHERTEXT_HEX);

        let ciphertext = encrypt(&key, &iv, &plaintext).unwrap();
        assert_eq!(ciphertext, expected, "IGE encryption does not match KAT");
        assert_eq!(decrypt(&key, &iv, &ciphertext).unwrap(), plaintext);
    }

    #[test]
    fn ige_roundtrip_for_various_lengths() {
        let key = [7u8; 32];
        let iv = [9u8; 32];
        for blocks in 1..=8 {
            let data: Vec<u8> = (0..blocks * BLOCK).map(|i| (i % 251) as u8).collect();
            let ciphertext = encrypt(&key, &iv, &data).unwrap();
            assert_ne!(ciphertext, data);
            assert_eq!(decrypt(&key, &iv, &ciphertext).unwrap(), data);
        }
    }

    #[test]
    fn ige_rejects_non_block_multiple() {
        let key = [0u8; 32];
        let iv = [0u8; 32];
        assert!(encrypt(&key, &iv, &[1, 2, 3]).is_err());
        assert!(decrypt(&key, &iv, &[1, 2, 3]).is_err());
    }

    #[test]
    fn ige_empty_input_is_allowed() {
        let key = [3u8; 32];
        let iv = [4u8; 32];
        assert!(encrypt(&key, &iv, &[]).unwrap().is_empty());
        assert!(decrypt(&key, &iv, &[]).unwrap().is_empty());
    }
}
