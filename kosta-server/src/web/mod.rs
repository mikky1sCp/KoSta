pub mod handlers;
pub mod ws;
pub mod models;
pub mod state;

use actix_web::{web, App, HttpServer, middleware::Logger, HttpResponse};
use actix_files::Files;
use actix_cors::Cors;
use std::sync::Arc;
use crate::db::Database;
use crate::session_cache::SessionCache;
use crate::web::state::WebState;
use crate::metrics; // <-- импорт метрик

pub fn run_web_server(db: Arc<Database>, cache: Arc<SessionCache>, host: &str, port: u16) -> std::io::Result<()> {
    let db = web::Data::new(db);
    let cache = web::Data::new(cache);
    let state = web::Data::new(WebState::new());
    let host = host.to_string();
    let port = port;

    actix_rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                .wrap(Cors::permissive())
                .wrap(Logger::default())
                .app_data(db.clone())
                .app_data(cache.clone())
                .app_data(state.clone())
                .service(
                    web::scope("/api")
                        .route("/login", web::post().to(handlers::login))
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
                .service(Files::new("/", "./web-ui").index_file("index.html"))
        })
        .bind((host.as_str(), port))?
        .run()
        .await
    })
}