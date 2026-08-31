// kosta/src/server_keys.rs
use ed25519_dalek::{Signature, Signer, Verifier, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use crate::error::KostaError;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use lazy_static::lazy_static; // <-- FIX: добавлен импорт

/// Пара ключей сервера (для подписи)
pub struct ServerKeypair {
    pub public: VerifyingKey,
    pub secret: SigningKey,
}

impl ServerKeypair {
    pub fn generate() -> Self {
        let mut csprng = OsRng {};
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        ServerKeypair {
            public: verifying_key,
            secret: signing_key,
        }
    }

    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let signature: Signature = self.secret.sign(data);
        signature.to_bytes().to_vec()
    }

    pub fn verify(public_key: &VerifyingKey, data: &[u8], sig: &[u8]) -> Result<(), KostaError> {
        let signature = Signature::from_slice(sig)
            .map_err(|_| KostaError::Protocol("Invalid signature format".into()))?;
        public_key.verify(data, &signature)
            .map_err(|_| KostaError::Protocol("Signature verification failed".into()))
    }
}

/// Загружает публичный ключ сервера из файла server_public_key.der (если есть),
/// иначе генерирует новый (для тестов, но тогда проверка подписи не пройдёт).
pub fn get_server_public_key() -> VerifyingKey {
    lazy_static! {
        static ref SERVER_PUBLIC_KEY: VerifyingKey = {
            let path = PathBuf::from("server_public_key.der");
            if path.exists() {
                let mut file = File::open(&path).expect("Failed to open server_public_key.der");
                let mut buf = [0u8; 32];
                file.read_exact(&mut buf).expect("Failed to read server_public_key.der");
                VerifyingKey::from_bytes(&buf).expect("Invalid public key")
            } else {
                // Если файла нет, генерируем новый ключ (для тестов)
                let pair = ServerKeypair::generate();
                pair.public
            }
        };
    }
    *SERVER_PUBLIC_KEY
}