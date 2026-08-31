// kosta-server/src/server.rs
use anyhow::{anyhow, Result};
use std::io::Cursor;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, error, debug};
use tokio::sync::broadcast;

use native_tls::TlsAcceptor;
use kosta_transport::{StreamTransport, Transport};
use kosta_transport::tls;
use kosta_core::tl::TlObject;
use kosta_core::tl::types::{TlRead, TlWrite};

use crate::handlers;
use crate::db::Database;
use crate::session_cache::SessionCache;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: String,
    pub use_tls: bool,
    pub cert_path: String,
    pub key_path: String,
}

pub async fn run_server(
    config: ServerConfig,
    db: Arc<Database>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on {}", addr);

    let tls_acceptor = if config.use_tls {
        let cert = std::fs::read(&config.cert_path)?;
        let key = std::fs::read(&config.key_path)?;
        let pkcs12 = native_tls::Identity::from_pkcs8(&cert, &key)?;
        Some(TlsAcceptor::new(pkcs12)?)
    } else {
        None
    };

    let session_cache = Arc::new(SessionCache::new(1000, db.clone()));

    let mut handles = Vec::new();

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (stream, peer) = match accept_result {
                    Ok(pair) => pair,
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                        continue;
                    }
                };
                info!("New connection from {}", peer);
                let cache = session_cache.clone();
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

                let handle = tokio::task::spawn_blocking(move || {
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
                            if let Err(e) = handle_client(&mut *transport, cache, db) {
                                error!("Client {} error: {}", peer, e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to establish transport for {}: {}", peer, e);
                        }
                    }
                });
                handles.push(handle);
            }
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received, stopping accept new connections.");
                break;
            }
        }
    }

    // Закрываем слушатель, чтобы освободить порт
    drop(listener);
    info!("Waiting for existing connections to finish...");

    // Ждём завершения всех обработчиков (с таймаутом)
    for handle in handles {
        tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await??;
    }

    info!("All connections closed.");
    Ok(())
}

// -----------------------------------------------------------------------------
// Обработчик клиента (синхронный, блокирующий)
// -----------------------------------------------------------------------------
fn handle_client(transport: &mut dyn Transport, cache: Arc<SessionCache>, db: Arc<Database>) -> Result<()> {
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
        debug!("Received raw frame, size: {} bytes", raw.len());

        let auth_key_id = if raw.len() >= 8 {
            let id = i64::from_le_bytes(
                raw[0..8].try_into().map_err(|_| anyhow!("Invalid auth_key_id length"))?
            );
            if id == 0 {
                None
            } else {
                debug!("Extracted auth_key_id: {}", id);
                match cache.get_session(id) {
                    Ok(Some(_)) => Some(id),
                    _ => None,
                }
            }
        } else {
            None
        };
        debug!("auth_key_id: {:?}", auth_key_id);

        let (tl_data, crypto_ctx, _seq_no) = if let Some(id) = auth_key_id {
            let mut session_data = cache.get_session(id)?
                .ok_or(anyhow!("Session not found"))?;

            if raw.len() < 8 + 16 + 16 {
                error!("Message too short for encrypted data");
                continue;
            }
            let nonce: [u8; 16] = raw[8..24].try_into()
                .map_err(|_| anyhow!("Invalid nonce length"))?;
            let tag_start = raw.len() - 16;
            let tag: [u8; 16] = raw[tag_start..].try_into()
                .map_err(|_| anyhow!("Invalid tag length"))?;
            let ciphertext = &raw[24..tag_start];
            debug!("Decrypting message (nonce={:?}, tag_len={})", &nonce[..4], tag.len());

            let plaintext = match session_data.crypto.decrypt_incoming_server(&nonce, ciphertext, &tag) {
                Ok(p) => p,
                Err(e) => {
                    error!("Decryption failed: {}", e);
                    continue;
                }
            };
            debug!("Decrypted size: {} bytes", plaintext.len());

            let mut cursor = Cursor::new(&plaintext);
            let msg_id = i64::read_bytes(&mut cursor)?;
            let seq_no = i32::read_bytes(&mut cursor)?;
            debug!("Parsed msg_id={}, seq_no={}", msg_id, seq_no);

            // Проверка монотонности msg_id
            if msg_id <= session_data.last_recv_msg_id {
                error!("msg_id not monotonically increasing: {} <= {}", msg_id, session_data.last_recv_msg_id);
                continue;
            }

            // Защита от replay-атак
            {
                let mut recent = session_data.recent_msg_ids.lock().unwrap();
                if recent.contains(&msg_id) {
                    error!("Replay attack detected: msg_id {} already used", msg_id);
                    continue;
                }
                recent.put(msg_id, ());
            }

            // Проверка seq_no
            let expected_seq = session_data.recv_seq_no + 1;
            if seq_no != expected_seq {
                error!("Invalid seq_no: got {}, expected {}", seq_no, expected_seq);
                continue;
            }

            // Проверка временной метки
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let now_msg_id = (now.as_millis() as i64) << 32;
            let diff_ms = ((msg_id - now_msg_id) >> 32).abs();
            if diff_ms > 30_000 {
                error!("Message timestamp out of range: diff={}ms", diff_ms);
                continue;
            }

            // Обновляем состояние сессии
            session_data.recv_seq_no = seq_no;
            session_data.last_recv_msg_id = msg_id;
            db.update_session_recv_seq_and_last_msg(id, seq_no, msg_id)?;
            db.update_session_counters(id, session_data.crypto.send_counter, session_data.crypto.recv_counter)?;
            cache.update_session(id, session_data.clone())?;

            let tl_data = &plaintext[cursor.position() as usize..];
            (tl_data.to_vec(), Some(session_data.crypto.clone()), seq_no)
        } else {
            debug!("Plaintext message, size: {} bytes", raw.len());
            (raw, None, -1)
        };

        let obj = match TlObject::read_boxed(&mut Cursor::new(&tl_data)) {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to parse TL object: {}", e);
                continue;
            }
        };
        debug!("Parsed TL object: {:?}", obj);

        let response = handlers::handle_request(transport, obj, &*cache, &db, auth_key_id)?;

        if let Some(data) = response {
            debug!("Sending response, size: {} bytes", data.len());
            if let Some(mut crypto) = crypto_ctx {
                let (send_seq_no, msg_id_counter) = if let Some(id) = auth_key_id {
                    if let Ok(Some(mut session)) = cache.get_session(id) {
                        let seq = session.send_seq_no + 1;
                        session.send_seq_no = seq;
                        let counter = session.msg_id_counter;
                        session.msg_id_counter = counter.wrapping_add(1);
                        cache.update_session(id, session.clone())?;
                        db.update_session_send_state(id, seq, session.msg_id_counter)?;
                        (seq, counter)
                    } else {
                        (-1, 0)
                    }
                } else {
                    (-1, 0)
                };

                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                let time_part = (now.as_millis() as i64) << 32;
                let msg_id = time_part | (msg_id_counter as i64);

                let mut plain_with_header = Vec::new();
                msg_id.write_bytes(&mut plain_with_header)?;
                send_seq_no.write_bytes(&mut plain_with_header)?;
                plain_with_header.extend_from_slice(&data);

                let (nonce, ciphertext, tag) = crypto.encrypt_outgoing_server(&plain_with_header)?;
                if let Some(id) = auth_key_id {
                    if let Ok(Some(mut session)) = cache.get_session(id) {
                        session.crypto = crypto.clone();
                        session.send_counter = crypto.send_counter;
                        session.recv_counter = crypto.recv_counter;
                        db.update_session_counters(id, crypto.send_counter, crypto.recv_counter)?;
                        cache.update_session(id, session.clone())?;
                    }
                }
                let auth_key_id_val = auth_key_id.unwrap_or(0);
                let mut msg = Vec::with_capacity(8 + 16 + ciphertext.len() + 16);
                msg.extend_from_slice(&auth_key_id_val.to_le_bytes());
                msg.extend_from_slice(&nonce);
                msg.extend_from_slice(&ciphertext);
                msg.extend_from_slice(&tag);
                transport.send(&msg)?;
                debug!("Encrypted response sent");
            } else {
                transport.send(&data)?;
                debug!("Plain response sent successfully");
            }
        } else {
            debug!("No response to send");
        }
    }
}