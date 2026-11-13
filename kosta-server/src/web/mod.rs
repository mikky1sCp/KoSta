// kosta-server/src/web/mod.rs
pub mod auth;
pub mod client_addr;
pub mod handlers;
pub mod models;
pub mod state;
pub mod ws;

use crate::db::Database;
use crate::media;
use crate::metrics;
use crate::web::state::WebState;
use actix_files::Files;
use actix_web::{
    middleware::{DefaultHeaders, Logger},
    web, App, HttpResponse, HttpServer,
};
use std::sync::Arc;

pub fn run_web_server(
    db: Arc<Database>,
    host: &str,
    port: u16,
    trust_proxy: bool,
) -> std::io::Result<()> {
    // Один раз на процесс: от этого флага зависит, верить ли Forwarded.
    client_addr::set_trust_proxy(trust_proxy);
    let state = web::Data::new(WebState::new(db.clone()));
    let db = web::Data::new(db);
    let host = host.to_string();
    // Статику выносим в env: развёртыванию не нужно пересобирать бинарь
    // ради другого каталога с web-ui.
    let ui_dir = std::env::var("KOSTA_WEB_UI_DIR").unwrap_or_else(|_| "./web-ui/dist".to_string());
    // Bytes-экстрактор должен принять файл размером с media-лимит. Даём 1 МиБ
    // запаса, чтобы наш JSON-413 сработал раньше стандартной ошибки экстрактора.
    let max_payload_bytes = media::max_bytes().saturating_add(1024 * 1024) as usize;

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
                // Сырое тело загрузки медиа ограничено media-лимитом.
                .app_data(web::PayloadConfig::new(max_payload_bytes))
                .service(
                    web::scope("/api")
                        .route("/login", web::post().to(handlers::login))
                        .route("/logout", web::post().to(handlers::logout))
                        .service(
                            web::resource("/me")
                                .route(web::get().to(handlers::get_me))
                                .route(web::patch().to(handlers::update_me)),
                        )
                        .service(
                            web::resource("/me/avatar")
                                .route(web::post().to(handlers::upload_avatar)),
                        )
                        .service(
                            web::resource("/avatars/{user_id}")
                                .route(web::get().to(handlers::get_avatar)),
                        )
                        .route("/chats", web::get().to(handlers::get_chats))
                        // Статические сегменты регистрируем до "/chats/{id}":
                        // actix выбирает ресурс по порядку регистрации, иначе
                        // "private"/"group" попали бы в динамический сегмент.
                        .service(
                            web::resource("/chats/private")
                                .route(web::post().to(handlers::create_private_chat)),
                        )
                        .service(
                            web::resource("/chats/group")
                                .route(web::post().to(handlers::create_group_chat)),
                        )
                        .service(
                            web::resource("/chats/{id}")
                                .route(web::get().to(handlers::get_chat))
                                .route(web::patch().to(handlers::rename_chat)),
                        )
                        .service(
                            web::resource("/chats/{id}/participants")
                                .route(web::post().to(handlers::add_participant)),
                        )
                        .service(
                            web::resource("/chats/{id}/participants/{user_id}")
                                .route(web::delete().to(handlers::remove_participant)),
                        )
                        .service(
                            web::resource("/chats/{id}/read")
                                .route(web::post().to(handlers::mark_chat_read)),
                        )
                        .service(
                            web::resource("/chats/{id}/pin")
                                .route(web::post().to(handlers::pin_message)),
                        )
                        // Загрузка файла в диалог: два сегмента после /chats,
                        // поэтому с "/chats/{id}" не конфликтует.
                        .service(
                            web::resource("/chats/{id}/media")
                                .route(web::post().to(handlers::upload_media)),
                        )
                        .route("/history", web::get().to(handlers::get_history))
                        // Поиск по истории диалога (без учёта регистра).
                        .route("/search", web::get().to(handlers::search_history))
                        // Правка и удаление своего сообщения (мягкое удаление).
                        .service(
                            web::resource("/messages/{id}")
                                .route(web::patch().to(handlers::edit_message))
                                .route(web::delete().to(handlers::delete_message)),
                        )
                        .service(
                            web::resource("/messages/{id}/reactions")
                                .route(web::post().to(handlers::toggle_reaction)),
                        )
                        // Скачивание по id сообщения: путь хранения наружу не отдаётся.
                        .route(
                            "/media/{message_id}",
                            web::get().to(handlers::download_media),
                        )
                        .route("/users/lookup", web::get().to(handlers::lookup_user)),
                )
                // --- Health & Metrics ---
                .route(
                    "/health",
                    web::get().to(|| async {
                        HttpResponse::Ok().json(serde_json::json!({"status":"ok"}))
                    }),
                )
                .route(
                    "/metrics",
                    web::get().to(|| async {
                        match metrics::render_metrics() {
                            Ok(body) => HttpResponse::Ok()
                                .content_type("text/plain; version=0.0.4")
                                .body(body),
                            Err(e) => {
                                HttpResponse::InternalServerError().body(format!("Error: {}", e))
                            }
                        }
                    }),
                )
                .service(web::resource("/ws").to(ws::websocket_handler))
                .service(Files::new("/", ui_dir.as_str()).index_file("index.html"))
        })
        .bind((host.as_str(), port))?
        .run()
        .await
    })
}
