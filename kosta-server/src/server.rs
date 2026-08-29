// kosta-server/src/server.rs
use anyhow::{anyhow, Result};
use std::io::Cursor;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, error};

use native_tls::TlsAcceptor;
use kosta_transport::{StreamTransport, Transport}; // <-- добавили Transport
use kosta_transport::tls;
use kosta_core::tl::TlObject;
use kosta_core::tl::types::{Int128, Int256, TlRead, TlWrite};

use crate::handlers;
use crate::session_store::{new_session_store, SessionStore, SessionData};
use crate::db::Database;
use std::time::{SystemTime, UNIX_EPOCH};

// Структура конфига (уже определена в main, но для полноты дублируем)
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: String,
    pub use_tls: bool,
    pub cert_path: String,
    pub key_path: String,
}

pub async fn run_server(config: ServerConfig, db: Arc<Database>) -> Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on {}", addr);

    // Загружаем TLS акцептор, если нужно
    let tls_acceptor = if config.use_tls {
        let cert = std::fs::read(&config.cert_path)?;
        let key = std::fs::read(&config.key_path)?;
        let pkcs12 = native_tls::Identity::from_pkcs8(&cert, &key)?;
        Some(TlsAcceptor::new(pkcs12)?)
    } else {
        None
    };

    let session_store = new_session_store();

    // Загружаем сессии из БД
    {
        let mut store_guard = session_store.lock().unwrap();
        let sessions = db.load_all_sessions()?;
        info!("Loaded {} sessions from database", sessions.len());
        for (auth_key_id, user_id, server_salt, nonce_vec, server_nonce_vec, new_nonce_vec, recv_seq_no,
             auth_key_vec, cwk, cmk, swk, smk, last_msg_id, send_counter, recv_counter) in sessions {
            let nonce = Int128(nonce_vec.try_into().map_err(|_| anyhow!("Invalid nonce length"))?);
            let server_nonce = Int128(server_nonce_vec.try_into().map_err(|_| anyhow!("Invalid server_nonce length"))?);
            let new_nonce = Int256(new_nonce_vec.try_into().map_err(|_| anyhow!("Invalid new_nonce length"))?);
            let auth_key = kosta_crypto::AuthKey(auth_key_vec.try_into().map_err(|_| anyhow!("Invalid auth_key length"))?);
            let mut client_write_key = [0u8; 32];
            client_write_key.copy_from_slice(&cwk);
            let mut client_mac_key = [0u8; 32];
            client_mac_key.copy_from_slice(&cmk);
            let mut server_write_key = [0u8; 32];
            server_write_key.copy_from_slice(&swk);
            let mut server_mac_key = [0u8; 32];
            server_mac_key.copy_from_slice(&smk);
            let mut crypto = kosta_crypto::session_crypto::SessionCrypto {
                client_write_key,
                client_mac_key,
                server_write_key,
                server_mac_key,
                send_counter: 0,
                recv_counter: 0,
            };
            crypto.send_counter = send_counter;
            crypto.recv_counter = recv_counter;
            let session_data = SessionData {
                auth_key,
                auth_key_id,
                user_id,
                server_salt,
                nonce,
                server_nonce,
                new_nonce,
                crypto,
                recv_seq_no,
                last_recv_msg_id: last_msg_id,
                send_counter,
                recv_counter,
                send_seq_no: -1,
                msg_id_counter: 0,
            };
            store_guard.insert(auth_key_id, session_data);
        }
        info!("Loaded {} sessions into memory", store_guard.len());
    }

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("New connection from {}", peer);
        let store = session_store.clone();
        let db = Arc::clone(&db);
        let acceptor = tls_acceptor.clone();

        let std_stream = match stream.into_std() {
            Ok(s) => {
                s.set_nonblocking(false)?;
                s
            }
            Err(e) => {
                error!("Failed to convert to std stream: {}", e);
                continue;
            }
        };

        tokio::task::spawn_blocking(move || {
            // Оборачиваем в TLS, если включено
            let transport_result: Result<Box<dyn Transport>, anyhow::Error> = if let Some(ref acc) = acceptor {
                match tls::accept_tls(std_stream, acc) {
                    Ok(t) => Ok(Box::new(t) as Box<dyn Transport>),
                    Err(e) => Err(anyhow!("TLS handshake failed: {}", e)),
                }
            } else {
                Ok(Box::new(StreamTransport::new(std_stream)) as Box<dyn Transport>)
            };

            match transport_result {
                Ok(mut transport) => {
                    if let Err(e) = handle_client(&mut *transport, store, db) {
                        error!("Client {} error: {}", peer, e);
                    }
                }
                Err(e) => {
                    error!("Failed to establish transport for {}: {}", peer, e);
                }
            }
        });
    }
}

// Обработчик клиента, теперь принимает &mut dyn Transport
fn handle_client(transport: &mut dyn Transport, store: SessionStore, db: Arc<Database>) -> Result<()> {
    let mut auth_key_id_opt: Option<i64> = None;
    let mut crypto_ctx_opt: Option<kosta_crypto::session_crypto::SessionCrypto> = None;
    let mut recv_seq_no = -1;
    let mut last_recv_msg_id = 0;

    loop {
        let raw = match transport.recv() {
            Ok(data) => data,
            Err(e) => {
                if let kosta_transport::TransportError::Io(ref io_err) = e {
                    if io_err.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Ok(());
                    }
                }
                return Err(anyhow!("Recv error: {}", e));
            }
        };
        info!("Received raw frame, size: {} bytes", raw.len());

        // Определяем auth_key_id
        let auth_key_id = if raw.len() >= 8 {
            let id = i64::from_le_bytes(raw[0..8].try_into().unwrap());
            if id == 0 {
                None
            } else {
                info!("Extracted auth_key_id: {}", id);
                let store_guard = store.lock().unwrap();
                if store_guard.contains_key(&id) {
                    info!("Found session for auth_key_id: {}", id);
                    Some(id)
                } else {
                    info!("No session for auth_key_id: {}", id);
                    None
                }
            }
        } else {
            None
        };
        info!("auth_key_id: {:?}", auth_key_id);

        let (tl_data, crypto_ctx, seq_no) = if let Some(id) = auth_key_id {
            let mut store_guard = store.lock().unwrap();
            let session_data = store_guard.get_mut(&id).ok_or(anyhow!("Session not found"))?;

            if raw.len() < 8 + 16 + 16 {
                error!("Message too short for encrypted data");
                continue;
            }
            let mut nonce = [0u8; 16];
            nonce.copy_from_slice(&raw[8..24]);
            let tag_start = raw.len() - 16;
            let mut tag = [0u8; 16];
            tag.copy_from_slice(&raw[tag_start..]);
            let ciphertext = &raw[24..tag_start];
            info!("Decrypting: nonce={:?}, tag={:?}", nonce, tag);

            let plaintext = match session_data.crypto.decrypt_incoming_server(&nonce, ciphertext, &tag) {
                Ok(p) => p,
                Err(e) => {
                    error!("Decryption failed: {}", e);
                    continue;
                }
            };
            info!("Decrypted size: {} bytes", plaintext.len());

            let mut cursor = Cursor::new(&plaintext);
            let msg_id = i64::read_bytes(&mut cursor)?;
            let seq_no = i32::read_bytes(&mut cursor)?;
            info!("Parsed msg_id={}, seq_no={}", msg_id, seq_no);

            if msg_id <= session_data.last_recv_msg_id {
                error!("msg_id not monotonically increasing: {} <= {}", msg_id, session_data.last_recv_msg_id);
                continue;
            }
            session_data.last_recv_msg_id = msg_id;

            let expected_seq = session_data.recv_seq_no + 1;
            if seq_no != expected_seq {
                error!("Invalid seq_no: got {}, expected {}", seq_no, expected_seq);
                continue;
            }

            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let now_msg_id = (now.as_millis() as i64) << 32;
            let diff_ms = ((msg_id - now_msg_id) >> 32).abs();
            if diff_ms > 30_000 {
                error!("Message timestamp out of range: diff={}ms", diff_ms);
                continue;
            }

            session_data.recv_seq_no = seq_no;
            db.update_session_recv_seq_and_last_msg(id, seq_no, msg_id)?;
            db.update_session_counters(id, session_data.crypto.send_counter, session_data.crypto.recv_counter)?;

            let tl_data = &plaintext[cursor.position() as usize..];
            (tl_data.to_vec(), Some(session_data.crypto.clone()), seq_no)
        } else {
            info!("Plaintext message, size: {} bytes", raw.len());
            (raw, None, -1)
        };

        let obj = match TlObject::read_boxed(&mut Cursor::new(&tl_data)) {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to parse TL object: {}", e);
                continue;
            }
        };
        info!("Parsed TL object: {:?}", obj);

        let response = handlers::handle_request(transport, obj, store.clone(), &db, auth_key_id)?;

        if let Some(data) = response {
            info!("Sending response, size: {} bytes", data.len());
            if let Some(mut crypto) = crypto_ctx {
                // Генерируем msg_id и seq_no для ответа
                let mut send_seq_no = -1;
                let mut msg_id_counter = 0;
                if let Some(id) = auth_key_id {
                    let mut store_guard = store.lock().unwrap();
                    if let Some(session) = store_guard.get_mut(&id) {
                        send_seq_no = session.send_seq_no + 1;
                        session.send_seq_no = send_seq_no;
                        msg_id_counter = session.msg_id_counter;
                        session.msg_id_counter = session.msg_id_counter.wrapping_add(1);
                    }
                }
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                let time_part = (now.as_millis() as i64) << 32;
                let msg_id = time_part | (msg_id_counter as i64);

                let mut plain_with_header = Vec::new();
                msg_id.write_bytes(&mut plain_with_header)?;
                send_seq_no.write_bytes(&mut plain_with_header)?;
                plain_with_header.extend_from_slice(&data);

                let (nonce, ciphertext, tag) = crypto.encrypt_outgoing_server(&plain_with_header)?;
                if let Some(id) = auth_key_id {
                    let mut store_guard = store.lock().unwrap();
                    if let Some(session) = store_guard.get_mut(&id) {
                        session.crypto = crypto.clone();
                        session.send_counter = crypto.send_counter;
                        session.recv_counter = crypto.recv_counter;
                        db.update_session_counters(id, crypto.send_counter, crypto.recv_counter)?;
                    }
                }
                let auth_key_id_val = auth_key_id.unwrap_or(0);
                let mut msg = Vec::with_capacity(8 + 16 + ciphertext.len() + 16);
                msg.extend_from_slice(&auth_key_id_val.to_le_bytes());
                msg.extend_from_slice(&nonce);
                msg.extend_from_slice(&ciphertext);
                msg.extend_from_slice(&tag);
                transport.send(&msg)?;
                info!("Encrypted response sent");
            } else {
                transport.send(&data)?;
                info!("Plain response sent successfully");
            }
        } else {
            info!("No response to send");
        }
    }
}