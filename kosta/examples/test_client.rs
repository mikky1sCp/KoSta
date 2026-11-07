// kosta/examples/test_client.rs

//! Пример клиента, демонстрирующий полный цикл работы с KoSta сервером:
//! - подключение к серверу
//! - аутентификация (обмен ключами)
//! - регистрация нового пользователя
//! - отправка сообщения
//! - получение истории

use anyhow::Result;
use kosta::{ConnectOptions, Session};
use kosta_core::tl::constructors::{CreateGroup, SignUp, TlObject};
use kosta_core::tl::types::ChatId;
use tracing::info;
use rand::rngs::OsRng;
use rand::Rng;

fn main() -> Result<()> {
    // Инициализация логирования
    tracing_subscriber::fmt::init();

    // Параметры подключения (можно переопределить переменными окружения)
    let host = std::env::var("KOSTA_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("KOSTA_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let use_tls = matches!(
        std::env::var("KOSTA_USE_TLS").as_deref(),
        Ok("1") | Ok("true")
    );
    // ⚠️ только для локальной отладки с самоподписанным сертификатом
    let allow_invalid_certs = matches!(
        std::env::var("KOSTA_TLS_INSECURE").as_deref(),
        Ok("1") | Ok("true")
    );
    let timeout_secs = 30;

    info!(
        "Connecting to {}:{} (tls={}, verify_cert={})",
        host,
        port,
        use_tls,
        !allow_invalid_certs
    );
    let mut session = Session::connect(
        &host,
        port,
        ConnectOptions {
            use_tls,
            allow_invalid_certs,
            timeout_secs,
            ..Default::default()
        },
    )?;

    info!("Connected, starting authentication...");
    session.authenticate()?;
    info!("Authentication successful");

    // Регистрация пользователя. Телефон можно задать через KOSTA_PHONE,
    // чтобы потом зайти этим же пользователем в web-ui.
    let phone = std::env::var("KOSTA_PHONE")
        .unwrap_or_else(|_| format!("+{}", OsRng.gen::<u32>()));
    let password = std::env::var("KOSTA_PASSWORD").unwrap_or_else(|_| "testpass123".to_string());
    info!("Registering user with phone: {}", phone);
    let signup = TlObject::SignUp(SignUp {
        phone: phone.clone(),
        password: password.clone(),
    });
    session.send_tl(&signup)?;
    info!("User registered successfully");

    // Создаём диалог: писать можно только в тот диалог, где пользователь — участник.
    // Сервер возвращает DialogInfo с id созданного диалога.
    session.send_tl(&TlObject::CreateGroup(CreateGroup {
        title: "KoSta test".to_string(),
        participants: vec![],
    }))?;
    let dialog_id = match session.recv_tl()? {
        TlObject::DialogInfo(info) => info.id.0,
        other => return Err(anyhow::anyhow!("Expected DialogInfo, got {:?}", other)),
    };
    info!("Dialog created: id={}", dialog_id);

    // Отправка сообщения в созданный диалог
    let chat_id = ChatId(dialog_id);
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