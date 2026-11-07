//! Отдельный listener MTProto 2.0 (по умолчанию `127.0.0.1:8082`).
//!
//! Легаси-протокол продолжает жить на своём порту до этапа 6; этот модуль его
//! не трогает и не импортирует легаси-сессии. Web-API/WS тоже не задействован.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use native_tls::TlsAcceptor;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::db::Database;

pub mod connection;
pub mod handshake;
pub mod rpc;
pub mod session;

/// Конфигурация listener'а MTProto 2.0.
#[derive(Debug, Clone)]
pub struct MtprotoConfig {
    /// Хост.
    pub host: String,
    /// Порт.
    pub port: u16,
    /// Таймаут простоя соединения.
    pub idle_timeout_secs: u64,
    /// Максимум одновременных соединений.
    pub max_connections: usize,
    /// Использовать TLS (те же сертификаты, что у основного listener'а).
    pub use_tls: bool,
    /// Путь к PEM-сертификату.
    pub cert_path: String,
    /// Путь к PKCS#8 ключу.
    pub key_path: String,
}

/// Запускает listener MTProto 2.0.
pub async fn run_mtproto_server(
    config: MtprotoConfig,
    db: Arc<Database>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind MTProto listener on {addr}"))?;
    info!("MTProto 2.0 listening on {}", addr);

    let tls_acceptor = if config.use_tls {
        let cert = std::fs::read(&config.cert_path)
            .with_context(|| format!("failed to read {}", config.cert_path))?;
        let key = std::fs::read(&config.key_path)
            .with_context(|| format!("failed to read {}", config.key_path))?;
        let identity = native_tls::Identity::from_pkcs8(&cert, &key)
            .context("failed to build TLS identity for MTProto listener")?;
        Some(TlsAcceptor::new(identity).context("failed to create TLS acceptor")?)
    } else {
        None
    };

    let store = Arc::new(session::SessionStore::new(db.clone()));

    // Чистим брошенные сессии MTProto 2.0 раз в час.
    {
        let cleanup_db = Arc::clone(&db);
        tokio::spawn(async move {
            let anonymous_idle = Duration::from_secs(3600);
            let user_idle = Duration::from_secs(30 * 24 * 3600);
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match cleanup_db.purge_stale_mtproto_sessions(
                    anonymous_idle.as_secs() as i64,
                    user_idle.as_secs() as i64,
                ) {
                    Ok(0) => {}
                    Ok(count) => info!("Purged {} stale MTProto sessions", count),
                    Err(error) => error!("Failed to purge MTProto sessions: {:#}", error),
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
                    Err(error) => {
                        error!("MTProto accept failed: {}", error);
                        continue;
                    }
                };

                let current = active_connections.fetch_add(1, Ordering::SeqCst) + 1;
                if current > max_connections {
                    active_connections.fetch_sub(1, Ordering::SeqCst);
                    warn!(
                        "Too many MTProto connections ({} > {}), rejecting {}",
                        current, max_connections, peer
                    );
                    drop(stream);
                    continue;
                }
                info!("New MTProto connection from {} (active: {})", peer, current);

                let std_stream = match stream.into_std() {
                    Ok(stream) => {
                        stream.set_nonblocking(false)?;
                        stream
                    }
                    Err(error) => {
                        error!("Failed to convert MTProto stream: {}", error);
                        active_connections.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                };

                if let Err(error) = std_stream
                    .set_read_timeout(Some(idle_timeout))
                    .and_then(|_| std_stream.set_write_timeout(Some(write_timeout)))
                {
                    error!("Failed to set timeouts for {}: {}", peer, error);
                    active_connections.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }

                let acceptor = tls_acceptor.clone();
                let store = Arc::clone(&store);
                let db = Arc::clone(&db);
                let active = Arc::clone(&active_connections);

                let handle = tokio::task::spawn_blocking(move || {
                    let conn = match acceptor {
                        Some(acceptor) => match acceptor.accept(std_stream) {
                            Ok(stream) => connection::Conn::Tls(stream),
                            Err(error) => {
                                error!("MTProto TLS handshake failed for {}: {}", peer, error);
                                active.fetch_sub(1, Ordering::SeqCst);
                                return;
                            }
                        },
                        None => connection::Conn::Plain(std_stream),
                    };

                    if let Err(error) =
                        connection::handle_connection(conn, store, db, peer, idle_timeout)
                    {
                        warn!("MTProto client {} error: {:#}", peer, error);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                });
                handles.push(handle);
                handles.retain(|handle| !handle.is_finished());
            }
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received, stopping MTProto listener.");
                break;
            }
        }
    }

    drop(listener);
    for handle in handles {
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    }
    info!("MTProto listener stopped.");
    Ok(())
}
