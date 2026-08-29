// =============================================================================
// kosta-transport/src/mock.rs
// =============================================================================
use crate::abridged;
use crate::error::TransportError;
use crate::transport::Transport;

/// Имитатор транспорта, работающий с буферами в памяти.
/// Отправленные данные накапливаются в `writer`, входящие добавляются
/// через `add_incoming`. Каждый вызов `recv` извлекает один Abridged‑фрейм
/// из начала `reader`.
pub struct MockTransport {
    reader: Vec<u8>,
    writer: Vec<u8>,
}

impl MockTransport {
    pub fn new() -> Self {
        MockTransport {
            reader: Vec::new(),
            writer: Vec::new(),
        }
    }

    /// Добавляет полученные «из сети» данные во входящий буфер.
    pub fn add_incoming(&mut self, data: &[u8]) {
        self.reader.extend_from_slice(data);
    }

    /// Извлекает все накопленные отправленные данные, очищая буфер.
    pub fn take_sent(&mut self) -> Vec<u8> {
        let mut sent = Vec::new();
        std::mem::swap(&mut sent, &mut self.writer);
        sent
    }
}

impl Transport for MockTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let frame = abridged::encode_frame(data);
        self.writer.extend_from_slice(&frame);
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        abridged::decode_frame_from_slice(&mut self.reader)
    }
}