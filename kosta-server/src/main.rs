// kosta-server/src/main.rs
use anyhow::Result;
use tracing_subscriber;
use std::sync::Arc;
use config::Config;

mod server;
mod handlers;
mod session_store;
mod server_keys;
mod db;
mod dh_params;
mod session_cache;   // добавлен

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

    tracing::info!("Starting KoSta server with config: {:?}", server_config);
    server::run_server(server_config, db).await?;
    Ok(())
}