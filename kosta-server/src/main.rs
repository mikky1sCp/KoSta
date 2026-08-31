use anyhow::Result;
use tracing_subscriber;
use std::sync::Arc;
use config::Config;
use tokio::signal;

mod rate_limiter;
mod metrics;
mod server;
mod handlers;
mod session_store;
mod server_keys;
mod db;
mod dh_params;
mod session_cache;
mod web;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::builder()
        .add_source(config::File::with_name("config"))
        .build()?;

    let server_config = server::ServerConfig {
        host: config.get_string("server.host")?,
        port: config.get_int("server.port")? as u16,
        db_path: config.get_string("server.db_path")?,
        use_tls: config.get_bool("server.use_tls")?,
        cert_path: config.get_string("server.cert_path")?,
        key_path: config.get_string("server.key_path")?,
    };

    server_keys::init_server_key();

    let db = Arc::new(db::Database::new(&server_config.db_path)?);
    let cache = Arc::new(session_cache::SessionCache::new(1000, db.clone()));

    // Запускаем веб-сервер с кэшем
    let web_db = db.clone();
    let web_cache = cache.clone();
    std::thread::spawn(move || {
        if let Err(e) = web::run_web_server(web_db, web_cache, "127.0.0.1", 8081) {
            eprintln!("Web server error: {}", e);
        }
    });

    tracing::info!("Starting KoSta server with config: {:?}", server_config);

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server_handle = tokio::spawn(server::run_server(server_config, db, shutdown_rx));

    signal::ctrl_c().await?;
    tracing::info!("Received SIGINT, initiating graceful shutdown...");
    let _ = shutdown_tx.send(());

    if let Err(e) = tokio::time::timeout(tokio::time::Duration::from_secs(5), server_handle).await {
        tracing::error!("Shutdown timeout: {:?}", e);
    }

    tracing::info!("Server gracefully shut down.");
    Ok(())
}