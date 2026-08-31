// kosta-server/src/server_keys.rs
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use rand::rngs::OsRng;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use anyhow::Result;
use lazy_static::lazy_static;

pub struct ServerKeypair {
    pub public: VerifyingKey,
    pub secret: SigningKey,
}

impl ServerKeypair {
    pub fn load_or_generate() -> Result<Self> {
        let path = PathBuf::from("server_key.der");
        if path.exists() {
            let mut file = File::open(&path)?;
            let mut buf = [0u8; 32];
            file.read_exact(&mut buf)?;
            let signing_key = SigningKey::from_bytes(&buf);
            let verifying_key = signing_key.verifying_key();
            Ok(ServerKeypair {
                public: verifying_key,
                secret: signing_key,
            })
        } else {
            let mut csprng = OsRng;
            let signing_key = SigningKey::generate(&mut csprng);
            let verifying_key = signing_key.verifying_key();
            let mut file = File::create(&path)?;
            file.write_all(signing_key.as_bytes())?;
            let mut pub_file = File::create("server_public_key.der")?;
            pub_file.write_all(verifying_key.as_bytes())?;
            Ok(ServerKeypair {
                public: verifying_key,
                secret: signing_key,
            })
        }
    }

    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let signature: Signature = self.secret.sign(data);
        signature.to_bytes().to_vec()
    }

    pub fn verify(public_key: &VerifyingKey, data: &[u8], sig: &[u8]) -> Result<()> {
        let signature = Signature::from_slice(sig)?;
        public_key.verify(data, &signature)?;
        Ok(())
    }
}

lazy_static! {
    static ref SERVER_KEYPAIR: ServerKeypair = ServerKeypair::load_or_generate().unwrap();
}

pub fn init_server_key() {
    let _ = &*SERVER_KEYPAIR;
}

pub fn get_server_keypair() -> &'static ServerKeypair {
    &SERVER_KEYPAIR
}