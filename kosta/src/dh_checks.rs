use num_bigint::BigUint;
use num_traits::{One, Zero};
use num_integer::Integer;
use rand::RngCore;

pub fn is_prime(n: &BigUint) -> bool {
    if n <= &BigUint::one() { return false; }
    if n % 2u32 == BigUint::zero() { return n == &BigUint::from(2u32); }
    if n % 3u32 == BigUint::zero() { return n == &BigUint::from(3u32); }

    let n_minus_1 = n - BigUint::one();
    let mut d = n_minus_1.clone();
    let mut s = 0u32;
    while d.is_even() {
        d /= 2u32;
        s += 1;
    }

    let mut rng = rand::thread_rng();
    let rounds = 20;

    for _ in 0..rounds {
        let a = if n <= &BigUint::from(4u32) {
            BigUint::from(2u32)
        } else {
            let range = n - BigUint::from(3u32);
            let mut bytes = vec![0u8; (range.bits() / 8 + 1) as usize];
            rng.fill_bytes(&mut bytes);
            BigUint::from_bytes_le(&bytes) % &range + BigUint::from(2u32)
        };

        let mut x = a.modpow(&d, n);
        if x == BigUint::one() || x == n_minus_1 {
            continue;
        }
        let mut cont = false;
        for _ in 0..s - 1 {
            x = (&x * &x) % n;
            if x == n_minus_1 {
                cont = true;
                break;
            }
        }
        if cont {
            continue;
        }
        return false;
    }
    true
}

pub fn validate_dh_params(dh_prime: &BigUint, g: &BigUint) -> Result<(), &'static str> {
    if !is_prime(dh_prime) {
        return Err("dh_prime is not prime");
    }
    let bits = dh_prime.bits();
    if bits < 2048 {
        return Err("dh_prime is too small (must be at least 2048 bits)");
    }
    if g <= &BigUint::one() || g >= &(dh_prime - BigUint::one()) {
        return Err("g must be in range (1, p-1)");
    }
    Ok(())
}

pub fn validate_public_value(public: &BigUint, p: &BigUint) -> Result<(), &'static str> {
    if public <= &BigUint::one() || public >= &(p - BigUint::one()) {
        return Err("Public key out of range");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prime() {
        let primes = [2u32, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47];
        for &p in &primes {
            assert!(is_prime(&BigUint::from(p)));
        }
        let composites = [1, 4, 6, 8, 9, 10, 12, 14, 15, 16, 18, 20, 21, 22, 24, 25];
        for &c in &composites {
            assert!(!is_prime(&BigUint::from(c as u32)));
        }
    }

    #[test]
    fn test_validate_dh_params() {
        let prime_hex = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF";
        let p = BigUint::parse_bytes(prime_hex.as_bytes(), 16).unwrap();
        let g = BigUint::from(2u32);
        assert!(validate_dh_params(&p, &g).is_ok());
    }
}