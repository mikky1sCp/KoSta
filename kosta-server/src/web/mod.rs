// kosta-server/src/web/mod.rs
pub mod auth;
pub mod handlers;
pub mod ws;
pub mod models;
pub mod state;

use actix_web::{
    middleware::{DefaultHeaders, Logger},
    web, App, HttpResponse, HttpServer,
};
use actix_files::Files;
use std::sync::Arc;
use crate::db::Database;
use crate::web::state::WebState;
use crate::metrics; // <-- импорт метрик

pub fn run_web_server(db: Arc<Database>, host: &str, port: u16) -> std::io::Result<()> {
    let state = web::Data::new(WebState::new(db.clone()));
    let db = web::Data::new(db);
    let host = host.to_string();
    // Статику выносим в env: развёртыванию не нужно пересобирать бинарь
    // ради другого каталога с web-ui.
    let ui_dir = std::env::var("KOSTA_WEB_UI_DIR").unwrap_or_else(|_| "./web-ui".to_string());

    actix_rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                // CORS не нужен: web-ui отдаётся с этого же origin, а permissive
                // открывал API любой странице в браузере.
                .wrap(
                    DefaultHeaders::new()
                        .add(("X-Content-Type-Options", "nosniff"))
                        .add(("Referrer-Policy", "no-referrer")),
                )
                .wrap(Logger::default())
                .app_data(db.clone())
                .app_data(state.clone())
                // Тело логина маленькое; 16 КиБ отсекает мегабайтные JSON.
                .app_data(web::JsonConfig::default().limit(16 * 1024))
                .service(
                    web::scope("/api")
                        .route("/login", web::post().to(handlers::login))
                        .route("/logout", web::post().to(handlers::logout))
                        .route("/chats", web::get().to(handlers::get_chats))
                        .route("/history", web::get().to(handlers::get_history))
                )
                // --- Health & Metrics ---
                .route("/health", web::get().to(|| async {
                    HttpResponse::Ok().json(serde_json::json!({"status":"ok"}))
                }))
                .route("/metrics", web::get().to(|| async {
                    match metrics::render_metrics() {
                        Ok(body) => HttpResponse::Ok()
                            .content_type("text/plain; version=0.0.4")
                            .body(body),
                        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
                    }
                }))
                .service(web::resource("/ws").to(ws::websocket_handler))
                .service(Files::new("/", ui_dir.as_str()).index_file("index.html"))
        })
        .bind((host.as_str(), port))?
        .run()
        .await
    })
}
