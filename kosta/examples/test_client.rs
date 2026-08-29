// kosta/examples/test_client.rs
use kosta::Session;
use kosta_core::tl::constructors::{SignUp, ChatId, TlObject};
use tracing::info;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Используем Session::connect для TCP (без TLS)
    let mut session = Session::connect("127.0.0.1", 8080, false, "")?;

    info!("Connected to server");
    info!("Starting authentication...");
    session.authenticate()?;
    info!("Authentication successful");

    let phone = format!("+{}", rand::random::<u32>());
    let password = "testpass123".to_string();
    let signup = TlObject::SignUp(SignUp {
        phone: phone.clone(),
        password: password.clone(),
    });
    session.send_tl(&signup)?;
    info!("User registered: {}", phone);

    let chat_id = ChatId(1);
    let text = "Hello, KoSta!".to_string();
    let msg = session.send_message(chat_id, text)?;
    info!("Message sent: {:?}", msg);

    let history = session.get_history(chat_id, 0, 10)?;
    info!("History: {} messages", history.messages.len());
    for msg in history.messages {
        info!("  [{:?}] {}: {}", msg.timestamp, msg.sender_id.0, msg.text);
    }

    Ok(())
}