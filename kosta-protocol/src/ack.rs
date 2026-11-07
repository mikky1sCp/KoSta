//! `msgs_ack` и кэш ответов на повторные сообщения.

use std::collections::{HashMap, VecDeque};

use crate::error::ProtocolError;
use crate::tl::{write_i32, write_i64, write_vector_len, Reader};

/// Идентификатор `msgs_ack#62d6b459`.
pub const MSGS_ACK_ID: u32 = 0x62d6_b459;
/// Ёмкость кэша ответов по умолчанию.
pub const DEFAULT_RESPONSE_CACHE_CAPACITY: usize = 1000;

/// Идентификаторы сообщений, которые нужно подтвердить.
#[derive(Debug, Default)]
pub struct AckTracker {
    pending: Vec<i64>,
}

impl AckTracker {
    /// Новый пустой трекер.
    pub fn new() -> Self {
        Self::default()
    }

    /// Помечает `msg_id` к подтверждению; повторная пометка игнорируется.
    ///
    /// Возвращает `true`, если идентификатор был добавлен впервые.
    pub fn mark(&mut self, msg_id: i64) -> bool {
        if self.pending.contains(&msg_id) {
            return false;
        }
        self.pending.push(msg_id);
        true
    }

    /// Нет ли отложенных подтверждений.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Количество отложенных подтверждений.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Собирает `msgs_ack`, если есть что подтверждать.
    pub fn build(&self) -> Option<Vec<u8>> {
        if self.pending.is_empty() {
            return None;
        }
        build_msgs_ack(&self.pending).ok()
    }

    /// Забирает все накопленные идентификаторы.
    pub fn drain(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.pending)
    }
}

/// Собирает TL-объект `msgs_ack`.
pub fn build_msgs_ack(msg_ids: &[i64]) -> Result<Vec<u8>, ProtocolError> {
    if msg_ids.is_empty() {
        return Err(ProtocolError::Malformed(
            "msgs_ack with an empty message list".into(),
        ));
    }
    let mut out = Vec::new();
    write_i32(&mut out, MSGS_ACK_ID as i32);
    write_vector_len(&mut out, msg_ids.len())?;
    for msg_id in msg_ids {
        write_i64(&mut out, *msg_id);
    }
    Ok(out)
}

/// Разбирает TL-объект `msgs_ack`.
pub fn parse_msgs_ack(body: &[u8]) -> Result<Vec<i64>, ProtocolError> {
    let mut reader = Reader::new(body);
    let id = reader.read_constructor_id()?;
    if id != MSGS_ACK_ID {
        return Err(ProtocolError::Malformed(format!(
            "expected msgs_ack 0x{MSGS_ACK_ID:08x}, got 0x{id:08x}"
        )));
    }
    let count = reader.read_vector_len()?;
    let mut ids = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        ids.push(reader.read_i64()?);
    }
    Ok(ids)
}

/// Кэш уже отправленных ответов: повторный запрос получает тот же ответ.
#[derive(Debug)]
pub struct ResponseCache {
    capacity: usize,
    map: HashMap<i64, Vec<u8>>,
    order: VecDeque<i64>,
}

impl ResponseCache {
    /// Создаёт кэш ёмкостью не меньше одного элемента.
    pub fn new(capacity: usize) -> Self {
        ResponseCache {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Запоминает ответ на `msg_id` (вытесняет самый старый при переполнении).
    pub fn insert(&mut self, msg_id: i64, response: Vec<u8>) {
        if self.map.insert(msg_id, response).is_none() {
            self.order.push_back(msg_id);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    /// Возвращает сохранённый ответ, если он ещё в кэше.
    pub fn get(&self, msg_id: i64) -> Option<&[u8]> {
        self.map.get(&msg_id).map(Vec::as_slice)
    }

    /// Количество сохранённых ответов.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Пуст ли кэш.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_tracker_marks_uniquely_and_drains() {
        let mut tracker = AckTracker::new();
        assert!(tracker.is_empty());
        assert!(tracker.build().is_none());

        assert!(tracker.mark(10));
        assert!(!tracker.mark(10));
        assert!(tracker.mark(20));
        assert_eq!(tracker.len(), 2);

        let body = tracker.build().unwrap();
        assert_eq!(parse_msgs_ack(&body).unwrap(), vec![10, 20]);

        let drained = tracker.drain();
        assert_eq!(drained, vec![10, 20]);
        assert!(tracker.is_empty());
    }

    #[test]
    fn empty_ack_is_rejected() {
        assert!(build_msgs_ack(&[]).is_err());
    }

    #[test]
    fn ack_rejects_wrong_object_and_truncation() {
        let mut body = build_msgs_ack(&[1, 2, 3]).unwrap();
        assert_eq!(parse_msgs_ack(&body).unwrap(), vec![1, 2, 3]);

        body[0] ^= 0xFF;
        assert!(parse_msgs_ack(&body).is_err());

        let body = build_msgs_ack(&[1, 2, 3]).unwrap();
        assert!(parse_msgs_ack(&body[..8]).is_err());
    }

    #[test]
    fn response_cache_returns_and_evicts() {
        let mut cache = ResponseCache::new(2);
        assert!(cache.is_empty());
        cache.insert(1, vec![1]);
        cache.insert(2, vec![2]);
        assert_eq!(cache.get(1), Some(&[1u8][..]));

        cache.insert(3, vec![3]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(1).is_none(), "oldest entry must be evicted");
        assert_eq!(cache.get(3), Some(&[3u8][..]));

        cache.insert(2, vec![22]);
        assert_eq!(cache.get(2), Some(&[22u8][..]));
        assert_eq!(cache.len(), 2);
    }
}
