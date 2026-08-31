use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::{One, Zero, ToPrimitive};
use rand::Rng;
use crate::error::KostaError;
use crate::dh_checks::is_prime;
use tracing::info;

// ---- Быстрая факторизация для u64 ----
fn factor_u64(n: u64) -> Option<(u64, u64)> {
    if n <= 1 {
        return None;
    }
    // Проверка чётности
    if n % 2 == 0 {
        let p = 2;
        let q = n / 2;
        if q > 1 {
            return Some((p, q));
        }
    }
    let limit = (n as f64).sqrt() as u64;
    let mut d = 3;
    while d <= limit {
        if n % d == 0 {
            let p = d;
            let q = n / d;
            if q > 1 {
                return Some((p, q));
            }
        }
        d += 2;
    }
    None   // n простое (или 1)
}

// ---- Решето Эратосфена (для малых простых) ----
fn primes_up_to(limit: usize) -> Vec<usize> {
    let mut sieve = vec![true; limit + 1];
    if limit > 0 {
        sieve[0] = false;
    }
    if limit > 1 {
        sieve[1] = false;
    }
    let sqrt_limit = (limit as f64).sqrt() as usize;
    for i in 2..=sqrt_limit {
        if sieve[i] {
            let mut j = i * i;
            while j <= limit {
                sieve[j] = false;
                j += i;
            }
        }
    }
    sieve.iter()
        .enumerate()
        .filter_map(|(i, &is_prime)| if is_prime { Some(i) } else { None })
        .collect()
}

lazy_static::lazy_static! {
    static ref SMALL_PRIMES: Vec<usize> = primes_up_to(100000);
}

/// Основная функция факторизации pq
pub fn factor_pq(n: &BigUint) -> Result<(Vec<u8>, Vec<u8>), KostaError> {
    info!("factor_pq: n = {:?}", n);
    if n <= &BigUint::one() {
        return Err(KostaError::Protocol("n must be > 1".into()));
    }

    // --- 1. Попытка быстрой факторизации для чисел, помещающихся в u64 ---
    if let Some(n_u64) = n.to_u64() {
        if let Some((p, q)) = factor_u64(n_u64) {
            let p_big = BigUint::from(p);
            let q_big = BigUint::from(q);
            if is_prime(&p_big) && is_prime(&q_big) {
                let (small, big) = if p < q { (p_big, q_big) } else { (q_big, p_big) };
                return Ok((small.to_bytes_be(), big.to_bytes_be()));
            }
        }
    }

    // --- 2. Пробное деление на малые простые ---
    info!("Trying trial division with small primes up to 100000");
    for &prime in SMALL_PRIMES.iter() {
        let p_big = BigUint::from(prime as u32);
        if n % &p_big == BigUint::zero() {
            let q_big = n / &p_big;
            if &p_big > &BigUint::one() && &q_big > &BigUint::one()
                && is_prime(&p_big) && is_prime(&q_big)
            {
                let (small, big) = if p_big < q_big {
                    (p_big.to_bytes_be(), q_big.to_bytes_be())
                } else {
                    (q_big.to_bytes_be(), p_big.to_bytes_be())
                };
                return Ok((small, big));
            }
        }
    }

    // --- 3. Алгоритм Полларда ρ (Brent) ---
    info!("Starting Pollard's rho (Brent)");
    let mut rng = rand::thread_rng();
    let max_attempts = 500;
    let max_iter = 200_000;

    for _attempt in 0..max_attempts {
        let c = BigUint::from(rng.gen_range(1u32..1000));
        let x0 = BigUint::from(rng.gen_range(2u32..100));

        let mut x = x0.clone();
        let mut y = x0.clone();
        let mut d = BigUint::one();
        let mut power = 1;
        let mut iteration = 0;

        while iteration < max_iter && d == BigUint::one() {
            if iteration == power {
                y = x.clone();
                power *= 2;
            }
            x = (&x * &x + &c) % n;
            let diff = if x > y { &x - &y } else { &y - &x };
            d = d.gcd(&diff);
            iteration += 1;
        }

        if d != BigUint::one() && &d != n {
            info!("Pollard's rho found factor d = {:?}", d);
            let p = d;
            let q = n / &p;
            if &p > &BigUint::one() && &q > &BigUint::one()
                && is_prime(&p) && is_prime(&q)
            {
                let (small, big) = if p < q {
                    (p.to_bytes_be(), q.to_bytes_be())
                } else {
                    (q.to_bytes_be(), p.to_bytes_be())
                };
                return Ok((small, big));
            }
        }
    }

    Err(KostaError::Protocol("Pollard's rho (Brent) failed to factor n".into()))
}