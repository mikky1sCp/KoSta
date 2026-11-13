// kosta-server/src/dh_params.rs
use num_bigint::BigUint;
use std::sync::LazyLock;

const DH_PRIME_2048_HEX: &str =
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF";

/// 2048-битное безопасное простое из RFC 3526 (group 14) и образующая `g = 2`.
///
/// Разбор ленивый: строка — константа в этом же файле, поэтому `expect` не
/// может сработать на сколько-нибудь валидной сборке.
pub static DH_PRIME: LazyLock<BigUint> = LazyLock::new(|| {
    BigUint::parse_bytes(DH_PRIME_2048_HEX.as_bytes(), 16).expect("hardcoded DH prime is valid hex")
});
pub static DH_G: LazyLock<BigUint> = LazyLock::new(|| BigUint::from(2u32));

#[cfg(test)]
mod tests {
    use super::*;
    use kosta_crypto::dh_checks::validate_dh_params;

    #[test]
    fn server_prime_passes_client_validation() {
        // Клиент проверяет присланные сервером параметры этими же правилами,
        // поэтому серверный prime обязан им соответствовать (2048 бит, p ≡ 7
        // mod 8, простое). Тест ловит рассинхрон сервера и `dh_checks`.
        validate_dh_params(&DH_PRIME, &DH_G)
            .expect("серверный DH prime должен проходить клиентскую проверку");
    }
}
