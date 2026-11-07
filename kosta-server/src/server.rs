// kosta-server/src/server.rs
use anyhow::{anyhow, Context, Result};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, error, debug, warn};
use tokio::sync::broadcast;

use native_tls::TlsAcceptor;
use kosta_transport::{StreamTransport, Transport};
use kosta_transport::tls;
use kosta_core::tl::TlObject;
use kosta_core::tl::types::{TlRead, TlWrite};

use crate::handlers;
use crate::db::Database;
use crate::rate_limiter::{check_limit, TCP_HANDSHAKE_LIMITER};
use crate::session_cache::SessionCache;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: String,
    pub use_tls: bool,
    pub cert_path: String,
    pub key_path: String,
    /// Через сколько секунд простоя закрывать соединение.
    pub idle_timeout_secs: u64,
    /// Максимум одновременных TCP-соединений.
    pub max_connections: usize,
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
        let cert = std::fs::read(&config.cert_path).with_context(|| {
            format!("не удалось прочитать cert_path={}", config.cert_path)
        })?;
        let key = std::fs::read(&config.key_path).with_context(|| {
            format!("не удалось прочитать key_path={}", config.key_path)
        })?;
        let identity = native_tls::Identity::from_pkcs8(&cert, &key)
            .context("не удалось собрать TLS identity (нужен PEM-сертификат и PKCS#8 ключ)")?;
        Some(TlsAcceptor::new(identity).context("не удалось создать TLS acceptor")?)
    } else {
        None
    };

    let session_cache = Arc::new(SessionCache::new(1000, db.clone()));

    // Очистка простаивающих сессий: и в БД, и в кэше. Только БД чистить
    // недостаточно — кэш хранил копию и персистил её обратно при следующем
    // обращении, «воскрешая» удалённую строку.
    {
        let cleanup_db = Arc::clone(&db);
        let cleanup_cache = Arc::clone(&session_cache);
        tokio::spawn(async move {
            let anonymous_idle = Duration::from_secs(3600);
            let user_idle = Duration::from_secs(30 * 24 * 3600);
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match cleanup_db.purge_stale_sessions(
                    anonymous_idle.as_secs() as i64,
                    user_idle.as_secs() as i64,
                ) {
                    Ok(0) => {}
                    Ok(n) => info!("Purged {} stale sessions", n),
                    Err(e) => error!("Failed to purge stale sessions: {:#}", e),
                }
                let pruned = cleanup_cache.prune_idle(anonymous_idle, user_idle);
                if pruned > 0 {
                    info!("Pruned {} idle cached sessions", pruned);
                }
            }
        });
    }

    let active_connections = Arc::new(AtomicUsize::new(0));
    let idle_timeout = Duration::from_secs(config.idle_timeout_secs.max(1));
    let write_timeout = Duration::from_secs(30);
    let max_connections = config.max_connections.max(1);

    let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

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

                // Каждое соединение занимает поток из пула spawn_blocking,
                // поэтому количество одновременных соединений ограничено.
                let current = active_connections.fetch_add(1, Ordering::SeqCst) + 1;
                if current > max_connections {
                    active_connections.fetch_sub(1, Ordering::SeqCst);
                    warn!(
                        "Too many connections ({} > {}), rejecting {}",
                        current, max_connections, peer
                    );
                    drop(stream);
                    continue;
                }
                info!("New connection from {} (active: {})", peer, current);

                let cache = session_cache.clone();
                let db = Arc::clone(&db);
                let acceptor = tls_acceptor.clone();
                let active = Arc::clone(&active_connections);

                let std_stream = match stream.into_std() {
                    Ok(s) => {
                        s.set_nonblocking(false)?;
                        s
                    }
                    Err(e) => {
                        error!("Failed to convert to std stream: {}", e);
                        active_connections.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                };

                // Таймауты обязательны: без них «залипший» клиент держит поток вечно.
                if let Err(e) = std_stream
                    .set_read_timeout(Some(idle_timeout))
                    .and_then(|_| std_stream.set_write_timeout(Some(write_timeout)))
                {
                    error!("Failed to set socket timeouts for {}: {}", peer, e);
                    active_connections.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }

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
                            if let Err(e) = handle_client(&mut *transport, cache, db, peer) {
                                error!("Client {} error: {}", peer, e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to establish transport for {}: {}", peer, e);
                        }
                    }

                    active.fetch_sub(1, Ordering::SeqCst);
                });
                handles.push(handle);
                // Иначе вектор завершённых задач растёт вместе с числом подключений.
                handles.retain(|h| !h.is_finished());
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
fn handle_client(
    transport: &mut dyn Transport,
    cache: Arc<SessionCache>,
    db: Arc<Database>,
    peer: std::net::SocketAddr,
) -> Result<()> {
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

        let tl_data = if let Some(id) = auth_key_id {
            if raw.len() < 8 + 16 + 16 {
                error!("Message too short for encrypted data");
                continue;
            }
            let nonce: [u8; 16] = raw[8..24]
                .try_into()
                .map_err(|_| anyhow!("Invalid nonce length"))?;
            let tag_start = raw.len() - 16;
            let tag: [u8; 16] = raw[tag_start..]
                .try_into()
                .map_err(|_| anyhow!("Invalid tag length"))?;
            let ciphertext = raw[24..tag_start].to_vec();
            debug!("Decrypting message (nonce={:?})", &nonce[..4]);

            // Расшифровка, проверки и фиксация состояния сессии — атомарно:
            // иначе два соединения с одним auth_key затирали счётчики друг друга.
            let outcome = cache.mutate_session(id, |session| {
                let plaintext = session
                    .crypto
                    .decrypt_incoming_server(&nonce, &ciphertext, &tag)
                    .map_err(|e| anyhow!("Decryption failed: {}", e))?;

                let mut cursor = Cursor::new(&plaintext);
                let msg_id = i64::read_bytes(&mut cursor)?;
                let seq_no = i32::read_bytes(&mut cursor)?;
                debug!("Parsed msg_id={}, seq_no={}", msg_id, seq_no);

                // Проверка монотонности msg_id
                if msg_id <= session.last_recv_msg_id {
                    return Err(anyhow!(
                        "msg_id not monotonically increasing: {} <= {}",
                        msg_id,
                        session.last_recv_msg_id
                    ));
                }

                // Защита от replay-атак
                {
                    let mut recent = session.recent_msg_ids.lock().unwrap();
                    if recent.contains(&msg_id) {
                        return Err(anyhow!("Replay attack detected: msg_id {}", msg_id));
                    }
                    recent.put(msg_id, ());
                }

                let expected_seq = session.recv_seq_no + 1;
                if seq_no != expected_seq {
                    return Err(anyhow!(
                        "Invalid seq_no: got {}, expected {}",
                        seq_no,
                        expected_seq
                    ));
                }

                // Проверка временной метки (старшие 32 бита msg_id — секунды)
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                let now_msg_id = (now.as_secs() as i64) << 32;
                if ((msg_id - now_msg_id) >> 32).abs() > 300 {
                    return Err(anyhow!("Message timestamp out of range"));
                }

                session.recv_seq_no = seq_no;
                session.last_recv_msg_id = msg_id;
                Ok(plaintext[cursor.position() as usize..].to_vec())
            });

            match outcome {
                Ok(Some(tl_data)) => tl_data,
                Ok(None) => {
                    error!("Session not found for auth_key_id {}", id);
                    continue;
                }
                Err(e) => {
                    error!("Rejected encrypted message: {:#}", e);
                    continue;
                }
            }
        } else {
            debug!("Plaintext message, size: {} bytes", raw.len());
            raw
        };

        let obj = match TlObject::read_boxed(&mut Cursor::new(&tl_data)) {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to parse TL object: {}", e);
                continue;
            }
        };
        debug!("Parsed TL object: {:?}", obj);

        // Рукопожатие — самая дорогая и при этом анонимная часть протокола
        // (ReqDHParams = 2048-битный modpow). Ограничиваем его по IP.
        if matches!(
            &obj,
            TlObject::ReqPq(_) | TlObject::ReqDHParams(_) | TlObject::SetClientDHParams(_)
        ) {
            if let Err(e) = check_limit(&TCP_HANDSHAKE_LIMITER, peer.ip().to_string()) {
                warn!("Handshake rate limit exceeded for {}: {}", peer, e);
                return Err(anyhow!("handshake rate limit exceeded for {}", peer));
            }
        }

        // Ошибка обработчика не должна обрывать соединение: логируем и живём дальше.
        let response = match handlers::handle_request(transport, obj, &cache, &db, auth_key_id, peer) {
            Ok(resp) => resp,
            Err(e) => {
                error!("Handler error: {:#}", e);
                continue;
            }
        };

        if let Some(data) = response {
            debug!("Sending response, size: {} bytes", data.len());
            if let Some(id) = auth_key_id {
                // Шифрование ответа и сдвиг send-счётчиков — одной атомарной операцией.
                let encrypted = match cache.mutate_session(id, |session| {
                    let seq_no = session.send_seq_no + 1;
                    let counter = session.msg_id_counter;
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                    // старшие 32 бита — unix-время в секундах (как в MTProto)
                    let msg_id = ((now.as_secs() as i64) << 32) | (counter as i64);

                    let mut plain_with_header = Vec::with_capacity(8 + data.len());
                    msg_id.write_bytes(&mut plain_with_header)?;
                    seq_no.write_bytes(&mut plain_with_header)?;
                    plain_with_header.extend_from_slice(&data);

                    let (nonce, ciphertext, tag) = session
                        .crypto
                        .encrypt_outgoing_server(&plain_with_header)?;

                    session.send_seq_no = seq_no;
                    session.msg_id_counter = counter.wrapping_add(1);
                    Ok((nonce, ciphertext, tag))
                }) {
                    Ok(value) => value,
                    Err(e) => {
                        error!("Failed to prepare encrypted response: {:#}", e);
                        continue;
                    }
                };

                match encrypted {
                    Some((nonce, ciphertext, tag)) => {
                        let mut msg = Vec::with_capacity(8 + 16 + ciphertext.len() + 16);
                        msg.extend_from_slice(&id.to_le_bytes());
                        msg.extend_from_slice(&nonce);
                        msg.extend_from_slice(&ciphertext);
                        msg.extend_from_slice(&tag);
                        transport.send(&msg)?;
                        debug!("Encrypted response sent");
                    }
                    None => error!("Session {} disappeared while sending response", id),
                }
            } else {
                transport.send(&data)?;
                debug!("Plain response sent successfully");
            }
        } else {
            debug!("No response to send");
        }
    }
}
