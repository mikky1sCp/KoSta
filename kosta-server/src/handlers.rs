// kosta-server/src/handlers.rs
use sha1::Digest;
use anyhow::{anyhow, Result};
use rand::Rng;
use rand::rngs::OsRng;
use rand::RngCore;
use std::io::Cursor;
use std::net::SocketAddr;
use num_bigint::BigUint;
use num_traits::One;
use kosta_core::tl::constructors::*;
use kosta_core::tl::types::{MessageId, UserId, UserStatus, DialogId};
use kosta_core::tl::types::{Int128, Int256, TlWrite};
use kosta_crypto::encrypted_inner::{encrypt_inner, decrypt_inner, tmp_key, tmp_key_from_nonce};
use kosta_crypto::padding;
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_transport::Transport;
use tracing::{info, warn};
use kosta_crypto::dh_checks::{is_prime, validate_public_value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};
use lazy_static::lazy_static;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::db::Database;
use crate::db::SessionRow;
use crate::rate_limiter::{check_limit, SIGNUP_LIMITER};
use crate::session_cache::SessionCache;
use crate::server_keys::get_server_keypair;
use crate::dh_params::{DH_PRIME, DH_G};

/// Максимальная длина текста сообщения (символы, не байты).
const MAX_MESSAGE_CHARS: usize = 4096;
/// Максимальная длина названия группы.
const MAX_GROUP_TITLE_CHARS: usize = 128;
/// Максимальное число участников группы (включая создателя).
const MAX_GROUP_PARTICIPANTS: usize = 200;

/// Пределы для пагинации истории. SQLite трактует отрицательный LIMIT как
/// «без лимита», поэтому значение ограничивается и в хендлере, и в БД.
fn clamp_history(limit: i32, offset: i32) -> (i32, i32) {
    (limit.clamp(1, 200), offset.max(0))
}

/// Телефон: необязательный "+", 5..=20 цифр, ничего кроме цифр.
fn is_valid_phone(phone: &str) -> bool {
    let digits = phone.strip_prefix('+').unwrap_or(phone);
    digits.len() <= 20 && digits.len() >= 5 && digits.chars().all(|c| c.is_ascii_digit())
}

/// Пароль: 8..=128 символов (bcrypt всё равно использует максимум 72 байта,
/// но ограничение длины защищает от гигантских тел запросов).
fn is_valid_password(password: &str) -> bool {
    (8..=128).contains(&password.chars().count())
}

// Хранилище для временных данных handshake с временем создания
type HandshakeRecord = (Int256, Vec<u8>, Instant);
type HandshakeStore = Mutex<HashMap<Vec<u8>, HandshakeRecord>>;

/// Сколько живёт незавершённое рукопожатие.
const HANDSHAKE_TTL: Duration = Duration::from_secs(60);

/// Предел числа «висящих» рукопожатий: каждая запись хранит серверный
/// DH-ключ, поэтому неограниченный рост — это утечка памяти и CPU.
const HANDSHAKE_STORE_MAX: usize = 10_000;

lazy_static! {
    static ref HANDSHAKE_STORE: HandshakeStore = Mutex::new(HashMap::new());
}

// Очистка устаревших записей
fn clean_handshake_store() {
    let now = Instant::now();
    let mut store = HANDSHAKE_STORE.lock().unwrap();
    store.retain(|_, (_, _, time)| now - *time < HANDSHAKE_TTL);
}

/// Сохраняет данные рукопожатия, предварительно вычистив просроченные
/// и не дав хранилищу вырасти выше предела.
fn remember_handshake(key: Vec<u8>, record: HandshakeRecord) -> Result<()> {
    let mut store = HANDSHAKE_STORE.lock().unwrap();
    let now = Instant::now();
    store.retain(|_, (_, _, time)| now - *time < HANDSHAKE_TTL);

    if store.len() >= HANDSHAKE_STORE_MAX && !store.contains_key(&key) {
        return Err(anyhow!(
            "too many pending handshakes ({}), retry later",
            store.len()
        ));
    }

    store.insert(key, record);
    Ok(())
}

// Генерация случайного простого числа (для сервера)
fn generate_random_prime(bits: usize) -> BigUint {
    let mut rng = OsRng;
    loop {
        let mut bytes = vec![0u8; bits.div_ceil(8)];
        rng.fill_bytes(&mut bytes);
        let num = BigUint::from_bytes_le(&bytes);
        let num = if num < BigUint::from(2u32) { BigUint::from(3u32) } else { num | BigUint::one() };
        if is_prime(&num) {
            return num;
        }
    }
}

// Вспомогательная функция для получения user_id из auth_key_id.
// user_id = 0 — системный пользователь: такие сессии ещё не прошли SignUp,
// и им нельзя разрешать создавать диалоги/писать сообщения.
fn get_user_id(auth_key_id: Option<i64>, db: &Database) -> Result<i64> {
    let id = auth_key_id.ok_or_else(|| anyhow!("Missing auth_key_id"))?;
    let user_id = db
        .get_session_user_id(id)?
        .ok_or_else(|| anyhow!("Session not found"))?;
    if user_id == 0 {
        return Err(anyhow!("Session is not signed up"));
    }
    Ok(user_id)
}

/// Проверяет, что пользователь является участником диалога.
/// Без этой проверки любой авторизованный клиент мог писать и читать в чужой dialog_id.
fn ensure_participant(db: &Database, dialog_id: i64, user_id: i64) -> Result<()> {
    let participants = db.get_dialog_participants(dialog_id)?;
    if participants.is_empty() {
        return Err(anyhow!("Dialog {} not found", dialog_id));
    }
    if !participants.contains(&user_id) {
        return Err(anyhow!("User {} is not a participant of dialog {}", user_id, dialog_id));
    }
    Ok(())
}

// ---------- Главный обработчик ----------
pub fn handle_request(
    _transport: &mut dyn Transport,
    obj: TlObject,
    cache: &SessionCache,
    db: &Database,
    auth_key_id: Option<i64>,
    peer: SocketAddr,
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
            handle_sign_up(req, db, cache, auth_key_id, peer)
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

    let keypair = get_server_keypair()?;
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

    let keypair = get_server_keypair()?;
    let expected_fingerprint = compute_key_fingerprint(&keypair.public);
    if req.public_key_fingerprint != expected_fingerprint {
        return Err(anyhow!("Unknown server public key fingerprint"));
    }

    let dh_prime = &*DH_PRIME;
    let g = &*DH_G;
    let a = kosta_crypto::dh::generate_private_key_in_range(dh_prime);
    let g_a = kosta_crypto::dh::compute_public_key(g, &a, dh_prime);

    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(&req.nonce.0);
    key.extend_from_slice(&req.server_nonce.0);
    remember_handshake(key, (new_nonce.clone(), a.to_bytes_be(), Instant::now()))?;

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

    let keypair = get_server_keypair()?;
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
        &SessionRow {
            user_id,
            server_salt: 0,
            nonce: &req.nonce.0,
            server_nonce: &req.server_nonce.0,
            new_nonce: &new_nonce.0,
            recv_seq_no: -1,
            auth_key: &auth_key.0,
            client_write_key: &crypto_ctx.client_write_key,
            client_mac_key: &crypto_ctx.client_mac_key,
            server_write_key: &crypto_ctx.server_write_key,
            server_mac_key: &crypto_ctx.server_mac_key,
            last_msg_id,
            send_counter,
            recv_counter,
            send_seq_no: -1,
            msg_id_counter: 0,
        },
    )?;

    // Сохраняем в кэш
    use crate::session_store::SessionData;
    let recent = Arc::new(Mutex::new(LruCache::new(
        NonZeroUsize::new(1000).expect("cache size must be non-zero")
    )));
    let session_data = SessionData {
        auth_key,
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
        last_used_at: Instant::now(),
    };
    cache.update_session(auth_key_id, session_data)?;

    {
        let mut store_handshake = HANDSHAKE_STORE.lock().unwrap();
        store_handshake.remove(&key);
    }

    let mut hasher = sha1::Sha1::new();
    hasher.update(shared_secret.to_bytes_le());
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
    peer: SocketAddr,
) -> Result<Option<Vec<u8>>> {
    // Регистрация создаёт пользователя и bcrypt-хеш — операция дорогая,
    // поэтому лимит отдельный от обычных сообщений.
    check_limit(&SIGNUP_LIMITER, peer.ip().to_string())
        .map_err(|e| anyhow!("SignUp rate limit exceeded: {}", e))?;
    if !is_valid_phone(&req.phone) {
        return Err(anyhow!("Invalid phone number"));
    }
    if !is_valid_password(&req.password) {
        return Err(anyhow!("Password must be 8..=128 characters"));
    }
    let session_id = auth_key_id.ok_or_else(|| {
        anyhow!("SignUp requires an established session (complete the DH handshake first)")
    })?;
    if db.get_user_id_by_phone(&req.phone)?.is_some() {
        return Err(anyhow!("Phone number is already registered"));
    }

    let user_id = db.create_user(&req.phone, &req.password)?;
    db.update_session_user_id(session_id, user_id)?;
    // Атомарно: иначе параллельно обрабатываемое сообщение могло
    // перезаписать user_id устаревшим клоном сессии.
    if let Err(e) = cache.mutate_session(session_id, |session| {
        session.user_id = user_id;
        Ok(())
    }) {
        warn!("Failed to attach user {} to session: {:#}", user_id, e);
    }
    info!("User registered: id={}", user_id);
    Ok(None)
}

// ---------- SendMessage (адаптирован под dialog_id) ----------
fn handle_send_message(req: SendMessage, db: &Database, auth_key_id: Option<i64>) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    // Предполагаем, что chat_id в запросе соответствует dialog_id
    let dialog_id = req.chat_id.0; // ChatId -> i64
    ensure_participant(db, dialog_id, user_id)?;

    // Пустые сообщения не сохраняем: они бесполезны и засоряют историю.
    let text = req.text.trim();
    if text.is_empty() {
        return Err(anyhow!("Message text is empty"));
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(anyhow!(
            "Message is too long: {} characters (max {})",
            text.chars().count(),
            MAX_MESSAGE_CHARS
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let msg_id = db.save_message(dialog_id, user_id, text, now, true, None, None)?;

    // Строим ответное сообщение (используем старую структуру Message с chat_id = dialog_id)
    let message = Message {
        id: MessageId(msg_id),
        chat_id: req.chat_id, // храним как диалог
        sender_id: UserId(user_id),
        text: text.to_string(),
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
    let user_id = get_user_id(auth_key_id, db)?; // проверяем авторизацию
    let dialog_id = req.chat_id.0;
    ensure_participant(db, dialog_id, user_id)?;

    let (limit, offset) = clamp_history(req.limit, req.offset);
    let rows = db.get_history(dialog_id, offset, limit)?;
    // `messages.is_outgoing` — это «как видел отправитель», одна колонка на
    // всех. Для истории правильная семантика зависит от зрителя: сообщение
    // исходящее, только если его отправил текущий пользователь.
    let messages: Vec<Message> = rows
        .into_iter()
        .map(|row| Message {
            id: MessageId(row.id),
            chat_id: req.chat_id,
            sender_id: UserId(row.sender_id),
            text: row.text,
            timestamp: row.timestamp,
            is_outgoing: row.sender_id == user_id,
            read: row.read,
            delivered: row.delivered,
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
    let user_id = get_user_id(auth_key_id, db)?;
    // Без этой проверки любой авторизованный клиент мог пометить delivered
    // чужое сообщение, зная только его id.
    let dialog_id = db
        .get_message_dialog_id(req.message_id.0)?
        .ok_or_else(|| anyhow!("Message {} not found", req.message_id.0))?;
    ensure_participant(db, dialog_id, user_id)?;
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
    if req.user_id.0 == user_id {
        return Err(anyhow!("Cannot create a private dialog with yourself"));
    }
    if !db.user_exists(req.user_id.0)? {
        return Err(anyhow!("Target user not found"));
    }
    let dialog_id = db.create_private_dialog(user_id, req.user_id.0)?;
    dialog_info_reply(dialog_id, user_id, db)
}

// ---------- CreateGroup ----------
fn handle_create_group(
    req: CreateGroup,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;

    let title = req.title.trim();
    if title.is_empty() || title.chars().count() > MAX_GROUP_TITLE_CHARS {
        return Err(anyhow!(
            "Group title must be 1..={} characters",
            MAX_GROUP_TITLE_CHARS
        ));
    }

    // Дедупликация и запрет добавлять создателя второй раз.
    let mut participants: Vec<i64> = Vec::new();
    for participant in &req.participants {
        let id = participant.0;
        if id == user_id || participants.contains(&id) {
            continue;
        }
        participants.push(id);
    }
    if participants.len() + 1 > MAX_GROUP_PARTICIPANTS {
        return Err(anyhow!(
            "Too many participants: max {}",
            MAX_GROUP_PARTICIPANTS
        ));
    }
    for id in &participants {
        if !db.user_exists(*id)? {
            return Err(anyhow!("Participant {} not found", id));
        }
    }

    let dialog_id = db.create_group_dialog(title, user_id, &participants)?;
    dialog_info_reply(dialog_id, user_id, db)
}

// ---------- AddGroupParticipant ----------
fn handle_add_group_participant(
    req: AddGroupParticipant,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let dialog_id = req.dialog_id.0;

    // Раньше любой участник мог добавлять людей в группу; теперь — только админ.
    let typ = db
        .get_dialog_type(dialog_id)?
        .ok_or_else(|| anyhow!("Dialog not found"))?;
    if typ != 1 {
        return Err(anyhow!("Not a group dialog"));
    }
    if !db.is_dialog_admin(dialog_id, user_id)? {
        return Err(anyhow!("Only group admins can add participants"));
    }
    if !db.user_exists(req.user_id.0)? {
        return Err(anyhow!("Target user not found"));
    }
    let participants = db.get_dialog_participants(dialog_id)?;
    if participants.len() >= MAX_GROUP_PARTICIPANTS {
        return Err(anyhow!(
            "Group is full: max {} participants",
            MAX_GROUP_PARTICIPANTS
        ));
    }
    if !participants.contains(&req.user_id.0) {
        db.add_participant(dialog_id, req.user_id.0)?;
    }
    dialog_info_reply(dialog_id, user_id, db)
}

// ---------- RemoveGroupParticipant ----------
fn handle_remove_group_participant(
    req: RemoveGroupParticipant,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let dialog_id = req.dialog_id.0;

    let meta = db
        .get_dialog_meta(dialog_id, user_id)?
        .ok_or_else(|| anyhow!("Dialog not found"))?;
    if !meta.is_group {
        return Err(anyhow!("Not a group dialog"));
    }
    if !meta.participants.contains(&user_id) {
        return Err(anyhow!("You are not a member of this group"));
    }

    if req.user_id.0 == user_id {
        // Выход из группы разрешён любому участнику.
        db.remove_participant(dialog_id, user_id)?;
    } else {
        if !db.is_dialog_admin(dialog_id, user_id)? {
            return Err(anyhow!("Only group admins can remove participants"));
        }
        if !meta.participants.contains(&req.user_id.0) {
            return Err(anyhow!("User is not a participant"));
        }
        db.remove_participant(dialog_id, req.user_id.0)?;
    }

    // Если ушёл последний админ — повышаем первого оставшегося участника.
    db.ensure_group_has_admin(dialog_id)?;
    // Если участников не осталось, диалог и его сообщения больше не нужны.
    // Ответить всё равно нужно: иначе клиент, ждущий DialogInfo, зависнет.
    if db.get_dialog_participants(dialog_id)?.is_empty() {
        let info = DialogInfo {
            id: DialogId(dialog_id),
            title: meta.title,
            is_group: true,
            participants: Vec::new(),
            last_message: None,
            unread_count: 0,
        };
        db.delete_empty_dialog(dialog_id)?;
        // Медиа-файлы удалённого диалога не должны оставаться на диске.
        // Ошибку только логируем: диалог уже удалён, ответ клиенту важнее.
        if let Err(e) = crate::media::delete_dialog(dialog_id) {
            warn!(
                "Failed to delete media of empty dialog {}: {:#}",
                dialog_id, e
            );
        }
        return dialog_info_response(info);
    }
    dialog_info_reply(dialog_id, user_id, db)
}

// ---------- GetDialogs ----------
fn handle_get_dialogs(
    _req: GetDialogs,
    db: &Database,
    auth_key_id: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let user_id = get_user_id(auth_key_id, db)?;
    let chats = db.get_user_chats(user_id)?;
    let mut dialog_infos = Vec::with_capacity(chats.len());
    for chat in chats {
        dialog_infos.push(build_dialog_info(chat.id, user_id, db)?);
    }
    let list = DialogList {
        dialogs: dialog_infos,
    };
    let obj = TlObject::DialogList(list);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

/// Формирует ответ `DialogInfo` для запрошенного диалога.
fn dialog_info_reply(dialog_id: i64, user_id: i64, db: &Database) -> Result<Option<Vec<u8>>> {
    let info = build_dialog_info(dialog_id, user_id, db)?;
    dialog_info_response(info)
}

/// Сериализует готовый `DialogInfo` в TL-ответ.
fn dialog_info_response(info: DialogInfo) -> Result<Option<Vec<u8>>> {
    let obj = TlObject::DialogInfo(info);
    let mut buf = Vec::new();
    obj.write_boxed(&mut buf)?;
    Ok(Some(buf))
}

// ---------- Вспомогательная функция для построения DialogInfo ----------
fn build_dialog_info(dialog_id: i64, user_id: i64, db: &Database) -> Result<DialogInfo> {
    // Тип и название берём из таблицы dialogs, а не угадываем по числу
    // участников: группа с одним участником остаётся группой, а сохранённое
    // название больше не теряется.
    let meta = db
        .get_dialog_meta(dialog_id, user_id)?
        .ok_or_else(|| anyhow!("Dialog {} not found", dialog_id))?;
    let participants: Vec<UserId> = meta.participants.into_iter().map(UserId).collect();
    Ok(DialogInfo {
        id: DialogId(meta.id),
        title: meta.title,
        is_group: meta.is_group,
        participants,
        last_message: None,
        unread_count: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_limits_are_clamped() {
        assert_eq!(clamp_history(0, 0), (1, 0));
        assert_eq!(clamp_history(-1, -5), (1, 0));
        assert_eq!(clamp_history(i32::MAX, 7), (200, 7));
        assert_eq!(clamp_history(50, 10), (50, 10));
    }

    #[test]
    fn phone_validation() {
        assert!(is_valid_phone("+79990000000"));
        assert!(is_valid_phone("12345"));
        assert!(!is_valid_phone("1234"));
        assert!(!is_valid_phone("+7999abc000"));
        assert!(!is_valid_phone(""));
        assert!(!is_valid_phone("+7 999 000 00 00"));
    }

    #[test]
    fn password_validation() {
        assert!(!is_valid_password("short"));
        assert!(is_valid_password("12345678"));
        assert!(is_valid_password(&"a".repeat(128)));
        assert!(!is_valid_password(&"a".repeat(129)));
    }
}
