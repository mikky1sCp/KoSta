use num_bigint::BigUint;
use rand::rngs::OsRng;
use rand::RngCore;

/// Генерирует случайное 2048-битное число для закрытого ключа
pub fn generate_private_key() -> BigUint {
    let mut rng = OsRng;
    let mut bytes = [0u8; 256]; // 256 байт = 2048 бит
    rng.fill_bytes(&mut bytes);
    BigUint::from_bytes_le(&bytes)
}

/// g^a mod p
pub fn compute_public_key(g: &BigUint, private: &BigUint, p: &BigUint) -> BigUint {
    g.modpow(private, p)
}

/// B^a mod p
pub fn compute_shared_secret(peer_public: &BigUint, private: &BigUint, p: &BigUint) -> BigUint {
    peer_public.modpow(private, p)
}

/// Заглушка проверки простоты (не используется в тестах)
pub fn is_prime(_n: &BigUint) -> bool {
    unimplemented!("prime check requires manual Miller-Rabin or external crate")
}