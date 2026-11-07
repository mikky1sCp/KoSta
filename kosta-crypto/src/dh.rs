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

/// Генерирует закрытый ключ в допустимом диапазоне `[2, p-2]`.
///
/// `generate_private_key() % p` теоретически может дать 0, 1 или p-1;
/// такие экспоненты дают тривиальный общий секрет (1 или g), поэтому
/// кандидат отбрасывается и генерируется заново.
pub fn generate_private_key_in_range(p: &BigUint) -> BigUint {
    let lower = BigUint::from(2u32);
    let upper = p - BigUint::from(2u32);
    loop {
        let candidate = generate_private_key() % p;
        if candidate >= lower && candidate <= upper {
            return candidate;
        }
    }
}

/// B^a mod p
pub fn compute_shared_secret(peer_public: &BigUint, private: &BigUint, p: &BigUint) -> BigUint {
    peer_public.modpow(private, p)
}

/// Общий секрет DH как `auth_key` MTProto: ровно 256 байт big-endian.
///
/// `BigUint::to_bytes_be` не дописывает ведущие нули, а MTProto требует
/// фиксированные 2048 бит — поэтому значение выравнивается слева нулями.
pub fn shared_secret_bytes(secret: &BigUint) -> [u8; 256] {
    let bytes = secret.to_bytes_be();
    let mut out = [0u8; 256];
    if bytes.len() >= 256 {
        out.copy_from_slice(&bytes[bytes.len() - 256..]);
    } else {
        out[256 - bytes.len()..].copy_from_slice(&bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_key_is_in_range() {
        for prime in [23u32, 101, 1009] {
            let p = BigUint::from(prime);
            for _ in 0..50 {
                let k = generate_private_key_in_range(&p);
                assert!(k >= BigUint::from(2u32));
                assert!(k <= &p - BigUint::from(2u32));
            }
        }
    }

    #[test]
    fn dh_roundtrip_matches_on_both_sides() {
        let p = BigUint::from(23u32);
        let g = BigUint::from(5u32);
        let a = generate_private_key_in_range(&p);
        let b = generate_private_key_in_range(&p);
        let pub_a = compute_public_key(&g, &a, &p);
        let pub_b = compute_public_key(&g, &b, &p);
        assert_eq!(
            compute_shared_secret(&pub_b, &a, &p),
            compute_shared_secret(&pub_a, &b, &p)
        );
    }

    #[test]
    fn shared_secret_bytes_are_left_padded() {
        let small = BigUint::from(0x0102u32);
        let bytes = shared_secret_bytes(&small);
        assert_eq!(bytes.len(), 256);
        assert_eq!(&bytes[254..], &[0x01, 0x02]);
        assert!(bytes[..254].iter().all(|b| *b == 0));

        let full = (BigUint::from(1u32) << 2040u32) + BigUint::from(7u32);
        let bytes = shared_secret_bytes(&full);
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[255], 0x07);
    }
}
