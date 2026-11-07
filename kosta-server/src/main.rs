use anyhow::Result;
use std::sync::Arc;
use config::Config;
use tokio::signal;

mod db;
mod dh_params;
mod media;
mod rate_limiter;
mod secrets;
mod server;
mod handlers;
mod session_store;
mod server_keys;
mod session_cache;
mod metrics;
mod web;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::builder()
        .add_source(config::File::with_name("config"))
        .build()?;

    // Медиа-хранилище инициализируем до web-сервера: хендлеры читают
    // глобальный экземпляр, а PayloadConfig берёт из него лимит.
    // KOSTA_MEDIA_DIR переопределяет каталог из конфига (удобно для тестов).
    let media_dir = std::env::var("KOSTA_MEDIA_DIR")
        .ok()
        .or_else(|| config.get_string("media.dir").ok())
        .unwrap_or_else(|| "media".to_string());
    let max_upload_mb = config
        .get_int("media.max_upload_mb")
        .unwrap_or(25)
        .max(1) as u64;
    media::init(&media_dir, max_upload_mb)?;

    let server_config = server::ServerConfig {
        host: config.get_string("server.host")?,
        port: config.get_int("server.port")? as u16,
        db_path: config.get_string("server.db_path")?,
        use_tls: config.get_bool("server.use_tls")?,
        cert_path: config.get_string("server.cert_path")?,
        key_path: config.get_string("server.key_path")?,
        idle_timeout_secs: config
            .get_int("server.idle_timeout_secs")
            .map(|v| v.max(1) as u64)
            .unwrap_or(300),
        max_connections: config
            .get_int("server.max_connections")
            .map(|v| v.max(1) as usize)
            .unwrap_or(256),
    };

    server_keys::init_server_key()?;

    // Мастер-ключ для шифрования секретов сессий в БД. Хранится отдельно
    // от базы, чтобы утечка файла БД не давала готовых сессий.
    let master_key_path = std::env::var("KOSTA_MASTER_KEY_PATH")
        .unwrap_or_else(|_| "session_master.key".to_string());
    let secrets = Arc::new(secrets::SecretsBox::load_or_create(std::path::Path::new(
        &master_key_path,
    ))?);

    let db = Arc::new(db::Database::new(&server_config.db_path, secrets)?);

    // Веб-сервер слушает свой хост/порт из конфига, а не захардкоженный 8081.
    let web_host = config
        .get_string("web.host")
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let web_port = config.get_int("web.port").map(|v| v as u16).unwrap_or(8081);
    // Путь к статике читается web-слоем из env (см. web/mod.rs).
    if let Ok(ui_dir) = config.get_string("web.ui_dir") {
        std::env::set_var("KOSTA_WEB_UI_DIR", ui_dir);
    }

    let web_db = db.clone();
    std::thread::spawn(move || {
        if let Err(e) = web::run_web_server(web_db, &web_host, web_port) {
            eprintln!("Web server error: {}", e);
        }
    });

    tracing::info!("Starting KoSta server with config: {:?}", server_config);

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let mut server_handle = tokio::spawn(server::run_server(server_config, db, shutdown_rx));

    // Раньше Result из spawn просто выбрасывался: если, например, не удавалось
    // собрать TLS identity, сервер молча не слушал порт, а процесс висел.
    // Теперь провал старта виден в логе и завершает процесс с ошибкой.
    tokio::select! {
        joined = &mut server_handle => {
            match joined {
                Ok(Ok(())) => {
                    tracing::warn!("Server task finished");
                }
                Ok(Err(e)) => {
                    tracing::error!("Server stopped with error: {:#}", e);
                    return Err(e);
                }
                Err(e) => {
                    tracing::error!("Server task failed: {}", e);
                    return Err(anyhow::anyhow!("server task failed: {}", e));
                }
            }
        }
        _ = signal::ctrl_c() => {
            tracing::info!("Received SIGINT, initiating graceful shutdown...");
            let _ = shutdown_tx.send(());
            if let Err(e) =
                tokio::time::timeout(tokio::time::Duration::from_secs(5), server_handle).await
            {
                tracing::error!("Shutdown timeout: {:?}", e);
            }
        }
    }

    tracing::info!("Server gracefully shut down.");
    Ok(())
}
