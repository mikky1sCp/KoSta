// kosta-server/src/metrics.rs
use prometheus::{register_counter, register_gauge, Counter, Gauge, Encoder, TextEncoder};
use std::sync::LazyLock;

// --- Метрики ---
pub static WS_CONNECTIONS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("kosta_ws_connections", "Number of active WebSocket connections")
        .expect("Failed to register WS_CONNECTIONS")
});

pub static MESSAGES_SENT: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!("kosta_messages_sent_total", "Total number of messages sent via WebSocket")
        .expect("Failed to register MESSAGES_SENT")
});

pub static AUTH_SUCCESS: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!("kosta_auth_success_total", "Total successful authentications")
        .expect("Failed to register AUTH_SUCCESS")
});

/// Генерирует ответ в формате Prometheus
pub fn render_metrics() -> Result<String, prometheus::Error> {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8(buffer).unwrap())
}