// kosta/examples/test_client.rs

//! Пример клиента, демонстрирующий полный цикл работы с KoSta сервером:
//! - подключение к серверу
//! - аутентификация (обмен ключами)
//! - регистрация нового пользователя
//! - отправка сообщения
//! - получение истории

use anyhow::Result;
use kosta::Session;
use kosta_core::tl::constructors::{SignUp, TlObject};
use kosta_core::tl::types::ChatId;
use tracing::{info, error};
use rand::rngs::OsRng;
use rand::Rng;

fn main() -> Result<()> {
    // Инициализация логирования
    tracing_subscriber::fmt::init();

    // Параметры подключения
    let host = "127.0.0.1";
    let port = 8080;
    let use_tls = false;
    let tls_domain = ""; // не используется при отключённом TLS
    let timeout_secs = 30;

    info!("Connecting to {}:{}", host, port);
    let mut session = Session::connect(host, port, use_tls, tls_domain, timeout_secs)?;

    info!("Connected, starting authentication...");
    session.authenticate()?;
    info!("Authentication successful");

    // Регистрация нового пользователя со случайным номером телефона
    let phone = format!("+{}", OsRng.gen::<u32>());
    let password = "testpass123".to_string();
    info!("Registering user with phone: {}", phone);
    let signup = TlObject::SignUp(SignUp {
        phone: phone.clone(),
        password: password.clone(),
    });
    session.send_tl(&signup)?;
    info!("User registered successfully");

    // Отправка сообщения в чат с ID = 1
    let chat_id = ChatId(1);
    let text = format!("Hello, KoSta! Это тестовое сообщение от {}", phone);
    info!("Sending message to chat {}: '{}'", chat_id.0, text);
    let sent_msg = session.send_message(chat_id, text)?;
    info!("Message sent: ID={}, timestamp={}", sent_msg.id.0, sent_msg.timestamp);

    // Запрос истории последних 10 сообщений
    info!("Requesting history for chat {}", chat_id.0);
    let history = session.get_history(chat_id, 0, 10)?;
    info!("History retrieved: {} messages", history.messages.len());
    for msg in history.messages {
        info!(
            "  [{}] User {}: {} (outgoing={}, read={}, delivered={})",
            msg.timestamp,
            msg.sender_id.0,
            msg.text,
            msg.is_outgoing,
            msg.read,
            msg.delivered
        );
    }

    info!("Test client finished successfully");
    Ok(())
}