// kosta-server/src/handlers.rs
use sha1::Digest;
use anyhow::{anyhow, Result};
use rand::Rng;
use rand::RngCore;
use rand::rngs::OsRng;
use std::io::Cursor;
use num_bigint::BigUint;
use num_traits::One;
use sha1;
use kosta_core::tl::constructors::*;
use kosta_core::tl::types::{Int128, Int256, TlWrite};
use kosta_crypto::encrypted_inner::{encrypt_inner, decrypt_inner, tmp_key, tmp_key_from_nonce};
use kosta_crypto::padding;
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_transport::Transport;   // <-- заменили TcpTransport на Transport
use tracing::{info, warn};
use kosta::dh_checks::{is_prime, validate_public_value};
use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;

use crate::db::Database;
use crate::session_store::{SessionStore, SessionData};
use crate::server_keys::get_server_keypair;
use crate::dh_params::{DH_PRIME, DH_G};

// Хранилище для временных данных handshake
lazy_static! {
    static ref HANDSHAKE_STORE: Mutex<HashMap<Vec<u8>, (Int256, Vec<u8>)>> = Mutex::new(HashMap::new());
}

pub fn handle_request(
    _transport: &mut dyn Transport,   // <-- изменено
    obj: TlObject,
    store: SessionStore,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    match obj {
        TlObject::ReqPq(req) => {
            info!("Received ReqPq");
            handle_req_pq(req)
        }
        TlObject::ReqDHParams(req) => {
            info!("Received ReqDHParams");
            handle_req_dh_params(req, store, db)
        }
        TlObject::SetClientDHParams(req) => {
            info!("Received SetClientDHParams");
            handle_set_client_dh_params(req, store, db)
        }
        TlObject::SignUp(req) => {
            info!("Received SignUp");
            handle_sign_up(req, db, &store, auth_key_id)
        }
        TlObject::SendMessage(req) => {
            info!("Received SendMessage");
            handle_send_message(req, db, auth_key_id)
        }
        TlObject::GetHistory(req) => {
            info!("Received GetHistory");
            handle_get_history(req, db)
        }
        TlObject::SendMessageAck(req) => {
            info!("Received SendMessageAck");
            handle_send_ack(req, db)
        }
        TlObject::UserStatusUpdate(req) => {
            info!("Received UserStatusUpdate");
            handle_user_status(req, db, auth_key_id)
        }
        _ => {
            warn!("Unsupported request: {:?}", obj);
            Err(anyhow!("Unsupported TL object"))
        }
    }
}

// Генерация случайного простого числа заданной битности
fn generate_random_prime(bits: usize) -> BigUint {
    let mut rng = rand::thread_rng();
    loop {
        let mut bytes = vec![0u8; (bits + 7) / 8];
        rng.fill_bytes(&mut bytes);
        let num = BigUint::from_bytes_le(&bytes);
        // Обеспечиваем нечётность и достаточную длину
        let num = if num < BigUint::from(2u32) { BigUint::from(3u32) } else { num | BigUint::one() };
        if is_prime(&num) {
            return num;
        }
    }
}

// ---------- ReqPq ----------
fn handle_req_pq(req: ReqPq) -> Result<Option<Vec<u8>>> {
    info!("Handling ReqPq");
    let mut rng = OsRng;
    let mut server_nonce_bytes = [0u8; 16];
    rng.fill(&mut server_nonce_bytes);
    let server_nonce = Int128(server_nonce_bytes);
    info!("Generated server_nonce: {:?}", server_nonce_bytes);

    // Генерируем два разных простых числа (32 бита)
    let p = generate_random_prime(32);
    let mut q = generate_random_prime(32);
    while q == p {
        q = generate_random_prime(32);
    }
    let pq_num = &p * &q;
    let pq = pq_num.to_bytes_be();
    info!("Generated pq: {} bytes", pq.len());

    let keypair = get_server_keypair();
    let fingerprint = compute_key_fingerprint(&keypair.public);
    info!("Fingerprint: {}", fingerprint);

    let res = ResPQ {
        nonce: req.nonce,
        server_nonce,
        pq,
        server_public_key_fingerprints: vec![fingerprint],
    };

    let obj = TlObject::ResPQ(res);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    info!("ResPQ serialized, size: {} bytes", buf.len());
    if buf.len() > 8 {
        info!("First 8 bytes of response: {:02x?}", &buf[..8]);
    }
    Ok(Some(buf))
}

fn compute_key_fingerprint(public_key: &ed25519_dalek::VerifyingKey) -> i64 {
    let mut hasher = sha1::Sha1::new();
    hasher.update(public_key.as_bytes());
    let hash = hasher.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(buf)
}

// ---------- ReqDHParams ----------
fn handle_req_dh_params(
    req: ReqDHParams,
    _store: SessionStore,
    _db: &Database,
) -> Result<Option<Vec<u8>>> {
    let (tmp_key_decrypt, nonce_gcm_decrypt) = tmp_key_from_nonce(&req.nonce, &req.server_nonce);
    let decrypted = decrypt_inner(&tmp_key_decrypt, &nonce_gcm_decrypt, &req.encrypted_data)?;
    let obj = TlObject::read_boxed(&mut Cursor::new(&decrypted))?;
    let pq_inner = match obj {
        TlObject::PqInnerData(data) => data,
        _ => return Err(anyhow!("Expected PqInnerData")),
    };

    if pq_inner.nonce != req.nonce || pq_inner.server_nonce != req.server_nonce {
        return Err(anyhow!("Nonce mismatch in PqInnerData"));
    }

    let new_nonce = pq_inner.new_nonce.clone();
    info!("Received new_nonce from client");

    let dh_prime = &*DH_PRIME;
    let g = &*DH_G;
    let a = kosta_crypto::dh::generate_private_key() % dh_prime;
    let g_a = kosta_crypto::dh::compute_public_key(g, &a, dh_prime);

    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(&req.nonce.0);
    key.extend_from_slice(&req.server_nonce.0);
    {
        let mut store = HANDSHAKE_STORE.lock().unwrap();
        store.insert(key, (new_nonce.clone(), a.to_bytes_be()));
    }

    let inner = ServerDHInnerData {
        nonce: req.nonce.clone(),
        server_nonce: req.server_nonce.clone(),
        g: 2,
        dh_prime: dh_prime.to_bytes_be(),
        g_a: g_a.to_bytes_be(),
        server_time: 0,
    };

    let mut inner_bytes = Vec::new();
    TlObject::ServerDHInnerData(inner).write_boxed(&mut inner_bytes)?;

    let (tmp_key_encrypt, nonce_gcm_encrypt) = tmp_key(&new_nonce, &req.server_nonce);
    let padded = padding::pad(&inner_bytes);
    let encrypted_answer = encrypt_inner(&tmp_key_encrypt, &nonce_gcm_encrypt, &padded)?;

    let keypair = get_server_keypair();
    let mut data_to_sign = Vec::new();
    req.nonce.write_bytes(&mut data_to_sign)?;
    req.server_nonce.write_bytes(&mut data_to_sign)?;
    encrypted_answer.write_bytes(&mut data_to_sign)?;
    let signature = keypair.sign(&data_to_sign);

    let response = ServerDHParamsOk {
        nonce: req.nonce,
        server_nonce: req.server_nonce,
        encrypted_answer,
        signature,
    };

    let obj = TlObject::ServerDHParamsOk(response);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- SetClientDHParams ----------
fn handle_set_client_dh_params(
    req: SetClientDHParams,
    store: SessionStore,
    db: &Database,
) -> Result<Option<Vec<u8>>> {
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(&req.nonce.0);
    key.extend_from_slice(&req.server_nonce.0);
    let (new_nonce, a_bytes) = {
        let store_handshake = HANDSHAKE_STORE.lock().unwrap();
        store_handshake.get(&key)
            .ok_or_else(|| anyhow!("No handshake data for this nonce pair"))?
            .clone()
    };
    let a = BigUint::from_bytes_be(&a_bytes);

    let (tmp_key_decrypt, nonce_gcm_decrypt) = tmp_key(&new_nonce, &req.server_nonce);
    let decrypted = decrypt_inner(&tmp_key_decrypt, &nonce_gcm_decrypt, &req.encrypted_data)?;
    let obj = TlObject::read_boxed(&mut Cursor::new(&decrypted))?;
    let client_inner = match obj {
        TlObject::ClientDHInnerData(data) => data,
        _ => return Err(anyhow!("Expected ClientDHInnerData")),
    };

    if client_inner.nonce != req.nonce || client_inner.server_nonce != req.server_nonce {
        return Err(anyhow!("Nonce mismatch in ClientDHInnerData"));
    }

    let dh_prime = &*DH_PRIME;
    let g_b = BigUint::from_bytes_be(&client_inner.g_b);
    if let Err(e) = validate_public_value(&g_b, dh_prime) {
        return Err(anyhow!("Invalid g_b: {}", e));
    }

    let shared_secret = kosta_crypto::dh::compute_shared_secret(&g_b, &a, dh_prime);

    let auth_key_id = compute_auth_key_id(&shared_secret);

    let crypto_ctx = SessionCrypto::new(
        &shared_secret.to_bytes_le(),
        &req.nonce.0,
        &req.server_nonce.0,
    );
    let auth_key = kosta_crypto::AuthKey::from_shared_secret(&shared_secret, &req.nonce.0, &req.server_nonce.0);

    let user_id = 0;

    let last_msg_id = 0;
    let send_counter = 0;
    let recv_counter = 0;

    db.save_session_full(
        auth_key_id,
        user_id,
        0,
        &req.nonce.0,
        &req.server_nonce.0,
        &new_nonce.0,
        -1,
        &auth_key.0,
        &crypto_ctx.client_write_key,
        &crypto_ctx.client_mac_key,
        &crypto_ctx.server_write_key,
        &crypto_ctx.server_mac_key,
        last_msg_id,
        send_counter,
        recv_counter,
    )?;

    let session_data = SessionData {
        auth_key,
        auth_key_id,
        user_id,
        server_salt: 0,
        nonce: req.nonce.clone(),
        server_nonce: req.server_nonce.clone(),
        new_nonce,
        crypto: crypto_ctx,
        recv_seq_no: -1,
        last_recv_msg_id: last_msg_id,
        send_counter,
        recv_counter,
        send_seq_no: -1,
        msg_id_counter: 0,
    };
    {
        let mut store_guard = store.lock().unwrap();
        store_guard.insert(auth_key_id, session_data);
        info!("Session stored in memory with auth_key_id: {}", auth_key_id);
    }

    {
        let mut store_handshake = HANDSHAKE_STORE.lock().unwrap();
        store_handshake.remove(&key);
    }

    let mut hasher = sha1::Sha1::new();
    hasher.update(&shared_secret.to_bytes_le());
    let hash = hasher.finalize();
    let mut new_nonce_hash1 = [0u8; 16];
    new_nonce_hash1.copy_from_slice(&hash[0..16]);
    let new_nonce_hash1 = Int128(new_nonce_hash1);

    let response = DHGenOk {
        nonce: req.nonce,
        server_nonce: req.server_nonce,
        new_nonce_hash1,
    };

    let obj = TlObject::DHGenOk(response);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

fn compute_auth_key_id(shared_secret: &BigUint) -> i64 {
    let secret_bytes = shared_secret.to_bytes_le();
    let mut hasher = sha1::Sha1::new();
    hasher.update(&secret_bytes);
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(bytes)
}

// ---------- SignUp ----------
fn handle_sign_up(
    req: SignUp,
    db: &Database,
    store: &SessionStore,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = db.create_user(&req.phone, &req.password)?;
    if let Some(id) = auth_key_id {
        let mut store_guard = store.lock().unwrap();
        if let Some(session) = store_guard.get_mut(&id) {
            session.user_id = user_id;
        }
        db.update_session_user_id(id, user_id)?;
    }
    info!("User registered with phone: {}", req.phone);
    Ok(None)
}

// ---------- SendMessage ----------
fn handle_send_message(req: SendMessage, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let auth_key_id = auth_key_id.ok_or_else(|| anyhow!("Missing auth_key_id"))?;
    let session = db.get_session(auth_key_id)?.ok_or_else(|| anyhow!("Session not found"))?;
    let user_id = session.0;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let msg_id = db.save_message(req.chat_id.0, user_id, &req.text, now, true)?;

    let message = Message {
        id: MessageId(msg_id),
        chat_id: req.chat_id,
        sender_id: UserId(user_id),
        text: req.text,
        timestamp: now,
        is_outgoing: true,
        read: false,
        delivered: false,
    };

    let obj = TlObject::Message(message);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- GetHistory ----------
fn handle_get_history(req: GetHistory, db: &Database) -> Result<Option<Vec<u8>>> {
    let rows = db.get_history(req.chat_id.0, req.offset, req.limit)?;
    let messages: Vec<Message> = rows
        .into_iter()
        .map(|(id, sender_id, text, ts, out, read, delivered)| Message {
            id: MessageId(id),
            chat_id: req.chat_id,
            sender_id: UserId(sender_id),
            text,
            timestamp: ts,
            is_outgoing: out,
            read,
            delivered,
        })
        .collect();

    let total_count = messages.len() as i32;
    let result = HistoryResult {
        messages,
        total_count,
    };
    let obj = TlObject::HistoryResult(result);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- SendMessageAck ----------
fn handle_send_ack(req: SendMessageAck, db: &Database) -> Result<Option<Vec<u8>>> {
    db.mark_message_delivered(req.message_id.0)?;
    Ok(None)
}

// ---------- UserStatusUpdate ----------
fn handle_user_status(req: UserStatusUpdate, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let auth_key_id = auth_key_id.ok_or_else(|| anyhow!("Missing auth_key_id"))?;
    let session = db.get_session(auth_key_id)?.ok_or_else(|| anyhow!("Session not found"))?;
    let user_id = session.0;
    if user_id != req.user_id.0 {
        return Err(anyhow!("Cannot update status for another user"));
    }
    let status_code = match req.status {
        UserStatus::Online => 0,
        UserStatus::Offline => 1,
        UserStatus::Typing => 2,
        UserStatus::Recently => 3,
        UserStatus::LastWeek => 4,
        UserStatus::LastMonth => 5,
    };
    db.set_user_status(user_id, status_code)?;
    Ok(None)
}