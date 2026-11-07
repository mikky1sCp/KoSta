//! Серверная часть DH-хендшейка MTProto 2.0 (свой вариант: Ed25519 вместо RSA).
//!
//! Схема: `req_pq` → `resPQ` (pq = произведение двух 32-битных простых, 64 бита)
//! → `req_DH_params` (клиент факторизует pq и присылает p/q; `encrypted_data`
//! зашифрован AES-GCM) → `server_DH_params_ok` (DH-2048, подпись Ed25519)
//! → `set_client_DH_params` → `dh_gen_ok`.
//!
//! После успеха создаётся сессия с сырым 2048-битным `auth_key` и
//! `auth_key_id = SHA1(auth_key)[12..20]`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use num_bigint::BigUint;
use num_traits::One;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use sha1::{Digest, Sha1};

use kosta_crypto::dh::{
    compute_public_key, compute_shared_secret, generate_private_key_in_range, shared_secret_bytes,
};
use kosta_crypto::dh_checks::{is_prime, validate_public_value};
use kosta_crypto::encrypted_inner::{
    decrypt_inner, encrypt_inner, tmp_key_bytes, tmp_key_from_nonce_bytes,
};
use kosta_crypto::handshake::new_nonce_hash1;
use kosta_crypto::mtproto2::auth_key_id;
use kosta_crypto::padding;
use kosta_tl::{
    Bytes, ClientDhInnerData, Int128, PqInnerData, Reader, ReqDhParams, ReqPq, ResPq,
    ServerDhInnerData, ServerDhParams, SetClientDhParams, SetClientDhParamsAnswer, TlObject,
    TlRead, TlWrite,
};

use crate::dh_params::{DH_G, DH_PRIME};
use crate::mtproto::session::SessionStore;
use crate::server_keys::get_server_keypair;

/// TTL незавершённого рукопожатия.
const HANDSHAKE_TTL: Duration = Duration::from_secs(60);
/// Максимум одновременно висящих рукопожатий.
const HANDSHAKE_STORE_MAX: usize = 10_000;

/// Состояние незавершённого рукопожатия.
#[derive(Clone)]
struct PendingHandshake {
    nonce: [u8; 16],
    server_nonce: [u8; 16],
    pq: u64,
    new_nonce: Option<[u8; 32]>,
    dh_private: Option<BigUint>,
    created: Instant,
}

type PendingStore = Mutex<HashMap<[u8; 32], PendingHandshake>>;

fn pending_store() -> &'static PendingStore {
    static STORE: OnceLock<PendingStore> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn handshake_key(nonce: &[u8; 16], server_nonce: &[u8; 16]) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(nonce);
    key[16..].copy_from_slice(server_nonce);
    key
}

fn cleanup_pending(now: Instant) {
    let mut store = pending_store().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, pending| now.duration_since(pending.created) < HANDSHAKE_TTL);
}

fn remember_pending(pending: PendingHandshake) -> Result<()> {
    let mut store = pending_store().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    store.retain(|_, item| now.duration_since(item.created) < HANDSHAKE_TTL);
    if store.len() >= HANDSHAKE_STORE_MAX
        && !store.contains_key(&handshake_key(&pending.nonce, &pending.server_nonce))
    {
        return Err(anyhow!("too many pending handshakes, retry later"));
    }
    store.insert(
        handshake_key(&pending.nonce, &pending.server_nonce),
        pending,
    );
    Ok(())
}

fn take_pending(nonce: &[u8; 16], server_nonce: &[u8; 16]) -> Result<PendingHandshake> {
    let store = pending_store().lock().unwrap_or_else(|e| e.into_inner());
    store
        .get(&handshake_key(nonce, server_nonce))
        .cloned()
        .ok_or_else(|| anyhow!("unknown handshake (nonce/server_nonce not found)"))
}

fn update_pending(
    nonce: &[u8; 16],
    server_nonce: &[u8; 16],
    new_nonce: [u8; 32],
    dh_private: BigUint,
) -> Result<()> {
    let mut store = pending_store().lock().unwrap_or_else(|e| e.into_inner());
    let pending = store
        .get_mut(&handshake_key(nonce, server_nonce))
        .ok_or_else(|| anyhow!("handshake disappeared"))?;
    pending.new_nonce = Some(new_nonce);
    pending.dh_private = Some(dh_private);
    Ok(())
}

fn remove_pending(nonce: &[u8; 16], server_nonce: &[u8; 16]) {
    let mut store = pending_store().lock().unwrap_or_else(|e| e.into_inner());
    store.remove(&handshake_key(nonce, server_nonce));
}

/// Отпечаток публичного ключа сервера (первые 64 бита SHA-1, LE i64).
pub fn server_key_fingerprint() -> Result<i64> {
    let keypair = get_server_keypair()?;
    let digest = Sha1::digest(keypair.public.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[12..20]);
    Ok(i64::from_le_bytes(bytes))
}

fn random_prime_32() -> BigUint {
    let mut rng = OsRng;
    loop {
        let candidate = rng.gen::<u32>() | 0x8000_0001;
        let number = BigUint::from(candidate);
        if is_prime(&number) {
            return number;
        }
    }
}

fn random_int128() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn serialize<T: TlWrite>(value: &T) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    value.write_bytes(&mut out)?;
    Ok(out)
}

/// Обрабатывает plaintext-объект хендшейка и возвращает ответ (если есть).
pub fn handle_plain_object(
    object: TlObject,
    store: &SessionStore,
    _peer: IpAddr,
) -> Result<Option<TlObject>> {
    cleanup_pending(Instant::now());
    match object {
        TlObject::ReqPq(request) => Ok(Some(TlObject::ResPq(handle_req_pq(request)?))),
        TlObject::ReqPqMulti(request) => Ok(Some(TlObject::ResPq(handle_req_pq(ReqPq {
            nonce: request.nonce,
        })?))),
        TlObject::ReqDhParams(request) => Ok(Some(TlObject::ServerDhParams(handle_req_dh_params(
            request,
        )?))),
        TlObject::SetClientDhParams(request) => Ok(Some(TlObject::SetClientDhParamsAnswer(
            handle_set_client_dh_params(request, store)?,
        ))),
        other => Err(anyhow!("unexpected plaintext TL object: {other:?}")),
    }
}

fn handle_req_pq(request: ReqPq) -> Result<ResPq> {
    let p = random_prime_32();
    let q = random_prime_32();
    let pq: u64 = (&p * &q)
        .try_into()
        .context("pq does not fit into u64 (must be 64-bit)")?;
    let server_nonce = random_int128();
    let pending = PendingHandshake {
        nonce: request.nonce.0,
        server_nonce,
        pq,
        new_nonce: None,
        dh_private: None,
        created: Instant::now(),
    };
    remember_pending(pending)?;

    Ok(ResPq {
        nonce: request.nonce,
        server_nonce: Int128(server_nonce),
        pq: Bytes(pq.to_be_bytes().to_vec()),
        server_public_key_fingerprints: vec![server_key_fingerprint()?],
    })
}

fn handle_req_dh_params(request: ReqDhParams) -> Result<ServerDhParams> {
    let pending = take_pending(&request.nonce.0, &request.server_nonce.0)?;

    let p = BigUint::from_bytes_be(&request.p.0);
    let q = BigUint::from_bytes_be(&request.q.0);
    if &p * &q != BigUint::from(pending.pq) {
        return Err(anyhow!("p*q does not match resPQ.pq"));
    }

    let (tmp_aes, tmp_nonce) = tmp_key_from_nonce_bytes(&request.nonce.0, &request.server_nonce.0);
    let decrypted = decrypt_inner(&tmp_aes, &tmp_nonce, &request.encrypted_data.0)
        .map_err(|e| anyhow!("failed to decrypt p_q_inner_data: {e}"))?;
    let inner = PqInnerData::read_bytes(&mut Reader::new(&decrypted))
        .map_err(|e| anyhow!("failed to parse p_q_inner_data: {e}"))?;

    if inner.nonce.0 != request.nonce.0 || inner.server_nonce.0 != request.server_nonce.0 {
        return Err(anyhow!("p_q_inner_data nonce mismatch"));
    }
    if BigUint::from_bytes_be(&inner.pq.0) != BigUint::from(pending.pq) {
        return Err(anyhow!("p_q_inner_data pq mismatch"));
    }
    if BigUint::from_bytes_be(&inner.p.0) != p || BigUint::from_bytes_be(&inner.q.0) != q {
        return Err(anyhow!("p_q_inner_data p/q mismatch"));
    }

    // new_nonce выбирает клиент (он же пришёл в p_q_inner_data).
    let new_nonce = inner.new_nonce.0;
    let dh_private = generate_private_key_in_range(&DH_PRIME);
    let g_a = compute_public_key(&DH_G, &dh_private, &DH_PRIME);

    let inner_data = ServerDhInnerData {
        nonce: request.nonce,
        server_nonce: request.server_nonce,
        g: 2,
        dh_prime: Bytes(DH_PRIME.to_bytes_be()),
        g_a: Bytes(g_a.to_bytes_be()),
        server_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i32)
            .unwrap_or(0),
    };
    let padded = padding::pad(&serialize(&inner_data)?);
    let (aes_key, aes_nonce) = tmp_key_bytes(&new_nonce, &request.server_nonce.0);
    let encrypted_answer =
        encrypt_inner(&aes_key, &aes_nonce, &padded).map_err(|e| anyhow!("encrypt answer: {e}"))?;

    let mut signed = Vec::with_capacity(16 + 16 + encrypted_answer.len());
    signed.extend_from_slice(&request.nonce.0);
    signed.extend_from_slice(&request.server_nonce.0);
    signed.extend_from_slice(&encrypted_answer);
    let signature = get_server_keypair()
        .context("server keypair is not initialized")?
        .sign(&signed);

    update_pending(
        &request.nonce.0,
        &request.server_nonce.0,
        new_nonce,
        dh_private,
    )?;

    Ok(ServerDhParams::ServerDhParamsOk {
        nonce: request.nonce,
        server_nonce: request.server_nonce,
        encrypted_answer: Bytes(encrypted_answer),
        signature: Bytes(signature),
    })
}

fn handle_set_client_dh_params(
    request: SetClientDhParams,
    store: &SessionStore,
) -> Result<SetClientDhParamsAnswer> {
    let pending = take_pending(&request.nonce.0, &request.server_nonce.0)?;
    let new_nonce = pending
        .new_nonce
        .ok_or_else(|| anyhow!("set_client_DH_params before req_DH_params"))?;
    let dh_private = pending
        .dh_private
        .ok_or_else(|| anyhow!("missing DH private key"))?;

    let (aes_key, aes_nonce) = tmp_key_bytes(&new_nonce, &request.server_nonce.0);
    let decrypted = decrypt_inner(&aes_key, &aes_nonce, &request.encrypted_data.0)
        .map_err(|e| anyhow!("failed to decrypt client_DH_inner_data: {e}"))?;
    let inner = ClientDhInnerData::read_bytes(&mut Reader::new(&decrypted))
        .map_err(|e| anyhow!("failed to parse client_DH_inner_data: {e}"))?;

    if inner.nonce.0 != request.nonce.0 || inner.server_nonce.0 != request.server_nonce.0 {
        return Err(anyhow!("client_DH_inner_data nonce mismatch"));
    }

    let g_b = BigUint::from_bytes_be(&inner.g_b.0);
    validate_public_value(&g_b, &DH_PRIME).map_err(|e| anyhow!("bad g_b: {e}"))?;

    let shared = compute_shared_secret(&g_b, &dh_private, &DH_PRIME);
    if shared <= BigUint::one() {
        return Err(anyhow!("degenerate shared secret"));
    }
    let auth_key = shared_secret_bytes(&shared);
    let key_id = auth_key_id(&auth_key);
    let salt = kosta_protocol::salt::generate_server_salt();
    store.create(key_id, auth_key, salt)?;
    remove_pending(&request.nonce.0, &request.server_nonce.0);

    Ok(SetClientDhParamsAnswer::DhGenOk {
        nonce: request.nonce,
        server_nonce: request.server_nonce,
        new_nonce_hash1: Int128(new_nonce_hash1(&new_nonce, &request.server_nonce.0)),
    })
}

/// Похож ли объект на дорогой шаг хендшейка (для rate limit по IP).
pub fn is_expensive_handshake_object(object: &TlObject) -> bool {
    matches!(
        object,
        TlObject::ReqPq(_)
            | TlObject::ReqPqMulti(_)
            | TlObject::ReqDhParams(_)
            | TlObject::SetClientDhParams(_)
    )
}
