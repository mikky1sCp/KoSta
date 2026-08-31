// kosta-server/src/handlers.rs
use sha1::Digest;
use anyhow::{anyhow, Result};
use rand::Rng;
use rand::rngs::OsRng;
use rand::RngCore;
use std::io::Cursor;
use num_bigint::BigUint;
use num_traits::One;
use sha1;
use kosta_core::tl::constructors::*;
use kosta_core::tl::types::{MessageId, UserId, UserStatus, DialogId};
use kosta_core::tl::types::{Int128, Int256, TlWrite};
use kosta_crypto::encrypted_inner::{encrypt_inner, decrypt_inner, tmp_key, tmp_key_from_nonce};
use kosta_crypto::padding;
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_transport::Transport;
use tracing::{info, warn};
use kosta::dh_checks::{is_prime, validate_public_value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};
use lazy_static::lazy_static;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::db::Database;
use crate::session_cache::SessionCache;
use crate::server_keys::get_server_keypair;
use crate::dh_params::{DH_PRIME, DH_G};

// Хранилище для временных данных handshake с временем создания
lazy_static! {
    static ref HANDSHAKE_STORE: Mutex<HashMap<Vec<u8>, (Int256, Vec<u8>, Instant)>> = Mutex::new(HashMap::new());
}

// Очистка устаревших записей
fn clean_handshake_store() {
    let now = Instant::now();
    let timeout = Duration::from_secs(60);
    let mut store = HANDSHAKE_STORE.lock().unwrap();
    store.retain(|_, (_, _, time)| now - *time < timeout);
}

// Генерация случайного простого числа (для сервера)
fn generate_random_prime(bits: usize) -> BigUint {
    let mut rng = OsRng;
    loop {
        let mut bytes = vec![0u8; (bits + 7) / 8];
        rng.fill_bytes(&mut bytes);
        let num = BigUint::from_bytes_le(&bytes);
        let num = if num < BigUint::from(2u32) { BigUint::from(3u32) } else { num | BigUint::one() };
        if is_prime(&num) {
            return num;
        }
    }
}

// Вспомогательная функция для получения user_id из auth_key_id
fn get_user_id(auth_key_id: Option<i64>, db: &Database) -> Result<i64> {
    let id = auth_key_id.ok_or_else(|| anyhow!("Missing auth_key_id"))?;
    let session = db.get_session(id)?.ok_or_else(|| anyhow!("Session not found"))?;
    Ok(session.0)
}

// ---------- Главный обработчик ----------
pub fn handle_request(
    _transport: &mut dyn Transport,
    obj: TlObject,
    cache: &SessionCache,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    match obj {
        // Существующие handshake
        TlObject::ReqPq(req) => {
            info!("Received ReqPq");
            handle_req_pq(req)
        }
        TlObject::ReqDHParams(req) => {
            info!("Received ReqDHParams");
            handle_req_dh_params(req, cache, db)
        }
        TlObject::SetClientDHParams(req) => {
            info!("Received SetClientDHParams");
            handle_set_client_dh_params(req, cache, db)
        }
        // Пользовательские
        TlObject::SignUp(req) => {
            info!("Received SignUp");
            handle_sign_up(req, db, cache, auth_key_id)
        }
        TlObject::SendMessage(req) => {
            info!("Received SendMessage");
            handle_send_message(req, db, auth_key_id)
        }
        TlObject::GetHistory(req) => {
            info!("Received GetHistory");
            handle_get_history(req, db, auth_key_id)
        }
        TlObject::SendMessageAck(req) => {
            info!("Received SendMessageAck");
            handle_send_ack(req, db, auth_key_id)
        }
        TlObject::UserStatusUpdate(req) => {
            info!("Received UserStatusUpdate");
            handle_user_status(req, db, auth_key_id)
        }
        // НОВЫЕ
        TlObject::CreatePrivateDialog(req) => {
            info!("Received CreatePrivateDialog");
            handle_create_private_dialog(req, db, auth_key_id)
        }
        TlObject::CreateGroup(req) => {
            info!("Received CreateGroup");
            handle_create_group(req, db, auth_key_id)
        }
        TlObject::AddGroupParticipant(req) => {
            info!("Received AddGroupParticipant");
            handle_add_group_participant(req, db, auth_key_id)
        }
        TlObject::RemoveGroupParticipant(req) => {
            info!("Received RemoveGroupParticipant");
            handle_remove_group_participant(req, db, auth_key_id)
        }
        TlObject::GetDialogs(req) => {
            info!("Received GetDialogs");
            handle_get_dialogs(req, db, auth_key_id)
        }
        _ => {
            warn!("Unsupported request: {:?}", obj);
            Err(anyhow!("Unsupported TL object"))
        }
    }
}

// ---------- ReqPq (без изменений) ----------
fn handle_req_pq(req: ReqPq) -> Result<Option<Vec<u8>>> {
    info!("Handling ReqPq");
    let mut rng = OsRng;
    let mut server_nonce_bytes = [0u8; 16];
    rng.fill(&mut server_nonce_bytes);
    let server_nonce = Int128(server_nonce_bytes);

    let p = generate_random_prime(32);
    let mut q = generate_random_prime(32);
    while q == p {
        q = generate_random_prime(32);
    }
    let pq_num = &p * &q;
    let pq = pq_num.to_bytes_be();
    let p_bytes = p.to_bytes_be();
    let q_bytes = q.to_bytes_be();
    info!("Generated pq ({} bytes), p ({} bytes), q ({} bytes)", pq.len(), p_bytes.len(), q_bytes.len());

    let keypair = get_server_keypair();
    let fingerprint = compute_key_fingerprint(&keypair.public);

    let res = ResPQ {
        nonce: req.nonce,
        server_nonce,
        pq,
        p: p_bytes,
        q: q_bytes,
        server_public_key_fingerprints: vec![fingerprint],
    };

    let obj = TlObject::ResPQ(res);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    info!("ResPQ serialized, size: {} bytes", buf.len());
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

// ---------- ReqDHParams (без изменений) ----------
fn handle_req_dh_params(
    req: ReqDHParams,
    _cache: &SessionCache,
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
        store.insert(key, (new_nonce.clone(), a.to_bytes_be(), Instant::now()));
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

// ---------- SetClientDHParams (без изменений, кроме создания сессии с новыми полями) ----------
fn handle_set_client_dh_params(
    req: SetClientDHParams,
    cache: &SessionCache,
    db: &Database,
) -> Result<Option<Vec<u8>>> {
    clean_handshake_store();

    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(&req.nonce.0);
    key.extend_from_slice(&req.server_nonce.0);
    let (new_nonce, a_bytes, _) = {
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

    let user_id = 0; // будет обновлено при регистрации

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
        -1,
        0,
    )?;

    // Сохраняем в кэш
    use crate::session_store::SessionData;
    let recent = Arc::new(Mutex::new(LruCache::new(
        NonZeroUsize::new(1000).expect("cache size must be non-zero")
    )));
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
        recent_msg_ids: recent,
    };
    cache.update_session(auth_key_id, session_data)?;

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

// ---------- SignUp (без изменений) ----------
fn handle_sign_up(
    req: SignUp,
    db: &Database,
    cache: &SessionCache,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = db.create_user(&req.phone, &req.password)?;
    if let Some(id) = auth_key_id {
        db.update_session_user_id(id, user_id)?;
        if let Ok(Some(mut session)) = cache.get_session(id) {
            session.user_id = user_id;
            cache.update_session(id, session)?;
        }
    }
    info!("User registered with phone: {}", req.phone);
    Ok(None)
}

// ---------- SendMessage (адаптирован под dialog_id) ----------
fn handle_send_message(req: SendMessage, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    // Предполагаем, что chat_id в запросе соответствует dialog_id
    let dialog_id = req.chat_id.0; // ChatId -> i64

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let msg_id = db.save_message(dialog_id, user_id, &req.text, now, true, None, None)?;

    // Строим ответное сообщение (используем старую структуру Message с chat_id = dialog_id)
    let message = Message {
        id: MessageId(msg_id),
        chat_id: req.chat_id, // храним как диалог
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

// ---------- GetHistory (адаптирован под dialog_id и с медиа-полями) ----------
fn handle_get_history(req: GetHistory, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let _user_id = get_user_id(auth_key_id, db)?; // просто проверяем авторизацию
    let dialog_id = req.chat_id.0;

    let rows = db.get_history(dialog_id, req.offset, req.limit)?;
    // rows: Vec<(id, sender_id, text, ts, out, read, delivered, media_path, media_type, media_size, is_media)>
    let messages: Vec<Message> = rows
        .into_iter()
        .map(|(id, sender_id, text, ts, out, read, delivered, _media_path, _media_type, _media_size, _is_media)| {
            Message {
                id: MessageId(id),
                chat_id: req.chat_id,
                sender_id: UserId(sender_id),
                text,
                timestamp: ts,
                is_outgoing: out,
                read,
                delivered,
            }
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
fn handle_send_ack(req: SendMessageAck, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let _user_id = get_user_id(auth_key_id, db)?;
    db.mark_message_delivered(req.message_id.0)?;
    Ok(None)
}

// ---------- UserStatusUpdate (обновлён для новой таблицы) ----------
fn handle_user_status(req: UserStatusUpdate, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
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

// =============================================================================
// НОВЫЕ ОБРАБОТЧИКИ (диалоги, группы)
// =============================================================================

// ---------- CreatePrivateDialog ----------
fn handle_create_private_dialog(
    req: CreatePrivateDialog,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let dialog_id = db.create_private_dialog(user_id, req.user_id.0)?;
    let info = build_dialog_info(dialog_id, user_id, db)?;
    let obj = TlObject::DialogInfo(info);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- CreateGroup ----------
fn handle_create_group(
    req: CreateGroup,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let participants: Vec<i64> = req.participants.iter().map(|u| u.0).collect();
    let dialog_id = db.create_group_dialog(&req.title, user_id, &participants)?;
    let info = build_dialog_info(dialog_id, user_id, db)?;
    let obj = TlObject::DialogInfo(info);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- AddGroupParticipant ----------
fn handle_add_group_participant(
    req: AddGroupParticipant,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let participants = db.get_dialog_participants(req.dialog_id.0)?;
    if !participants.contains(&user_id) {
        return Err(anyhow!("You are not a member of this group"));
    }
    db.add_participant(req.dialog_id.0, req.user_id.0)?;
    let info = build_dialog_info(req.dialog_id.0, user_id, db)?;
    let obj = TlObject::DialogInfo(info);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- RemoveGroupParticipant ----------
fn handle_remove_group_participant(
    req: RemoveGroupParticipant,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let participants = db.get_dialog_participants(req.dialog_id.0)?;
    if !participants.contains(&user_id) {
        return Err(anyhow!("You are not a member of this group"));
    }
    if user_id == req.user_id.0 {
        return Err(anyhow!("Cannot remove yourself from group"));
    }
    db.remove_participant(req.dialog_id.0, req.user_id.0)?;
    let info = build_dialog_info(req.dialog_id.0, user_id, db)?;
    let obj = TlObject::DialogInfo(info);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- GetDialogs ----------
fn handle_get_dialogs(
    _req: GetDialogs,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let dialogs_raw = db.get_user_dialogs(user_id)?;
    let mut dialog_infos = Vec::new();
    for (id, _title, _typ, _other_phone) in dialogs_raw {
        let info = build_dialog_info(id, user_id, db)?;
        dialog_infos.push(info);
    }
    let list = DialogList { dialogs: dialog_infos };
    let obj = TlObject::DialogList(list);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- Вспомогательная функция для построения DialogInfo ----------
fn build_dialog_info(dialog_id: i64, user_id: i64, db: &Database) -> Result<DialogInfo> {
    let participants_vec = db.get_dialog_participants(dialog_id)?;
    let participants: Vec<UserId> = participants_vec.into_iter().map(UserId).collect();
    let is_group = participants.len() > 2;
    let title = if is_group {
        format!("Group {}", dialog_id)
    } else {
        let other = participants.iter().find(|&&u| u.0 != user_id).map(|u| u.0).unwrap_or(0);
        let phone = if other != 0 {
            let conn = db.get_conn()?;
            let mut stmt = conn.prepare("SELECT phone FROM users WHERE id = ?1")?;
            let mut rows = stmt.query(rusqlite::params![other])?;
            if let Some(row) = rows.next()? {
                row.get(0)?
            } else {
                "Unknown".to_string()
            }
        } else {
            "Unknown".to_string()
        };
        phone
    };
    let last_message = None;
    let unread_count = 0;
    Ok(DialogInfo {
        id: DialogId(dialog_id),
        title,
        is_group,
        participants,
        last_message,
        unread_count,
    })
}