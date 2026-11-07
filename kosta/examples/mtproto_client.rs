//! E2E-проверка клиента MTProto 2.0:
//! хендшейк → signUp → createGroup → sendMessage → getHistory → getDialogs.
//!
//! Запускается против сервера на порту `KOSTA_PORT` (по умолчанию 8080).

use anyhow::{anyhow, Result};
use kosta::{ConnectOptions, MtprotoSession};
use kosta_tl::{ChatMessage, UserStatus};

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let host = std::env::var("KOSTA_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("KOSTA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let use_tls = matches!(
        std::env::var("KOSTA_USE_TLS").as_deref(),
        Ok("1") | Ok("true")
    );
    let allow_invalid_certs = matches!(
        std::env::var("KOSTA_TLS_INSECURE").as_deref(),
        Ok("1") | Ok("true")
    );

    let mut session = MtprotoSession::connect(
        &host,
        port,
        ConnectOptions {
            use_tls,
            allow_invalid_certs,
            timeout_secs: 30,
            ..Default::default()
        },
    )?;
    println!("connected to {host}:{port} (MTProto 2.0, tls={use_tls})");

    session.authenticate()?;
    println!(
        "handshake ok, auth_key_id={} salt={}",
        session.auth_key_id(),
        session.salt()
    );

    let phone =
        std::env::var("KOSTA_PHONE").unwrap_or_else(|_| format!("+{}", rand::random::<u32>()));
    let password = std::env::var("KOSTA_PASSWORD").unwrap_or_else(|_| "testpass123".to_string());
    let user_id = session.sign_up(&phone, &password)?;
    println!("signed up: user_id={user_id} phone={phone}");

    let dialog_id = session.create_group("KoSta MTProto test")?;
    println!("dialog created: {dialog_id}");

    let text = format!("Hello from MTProto 2.0, phone {phone}");
    let sent = session.send_message(dialog_id, &text)?;
    let sent_id = match &sent {
        ChatMessage::ChatMessage {
            id,
            text: sent_text,
            ..
        } => {
            if *sent_text != text {
                return Err(anyhow!("message text mismatch"));
            }
            println!("message sent: id={id}");
            *id
        }
        ChatMessage::ChatMessageEmpty => return Err(anyhow!("server returned an empty message")),
    };

    session.ack_message(sent_id)?;
    session.update_status(UserStatus::UserStatusOnline)?;

    let history = session.get_history(dialog_id, 0, 10)?;
    println!("history: {} messages", history.len());
    let found = history.iter().any(
        |message| matches!(message, ChatMessage::ChatMessage { text: other, .. } if *other == text),
    );
    if !found {
        return Err(anyhow!("sent message not found in history"));
    }

    let dialogs = session.get_dialogs()?;
    println!("dialogs: {}", dialogs.len());
    if !dialogs.iter().any(|dialog| dialog.id == dialog_id) {
        return Err(anyhow!("created dialog not found in dialog list"));
    }

    println!("MTProto 2.0 e2e OK");
    Ok(())
}
