//! `msg_id` и `seq_no` по правилам MTProto 2.0.
//!
//! * `msg_id` — 64 бита: старшие 32 — unix-время в секундах, младшие 32 —
//!   счётчик, сдвинутый на 2 бита, плюс бит чётности. Клиентские сообщения
//!   имеют `msg_id % 4 == 0`, серверные — `msg_id % 4 == 1`.
//! * `seq_no`: content-related сообщения — `2*n + 1`, сервисные — `2*n`,
//!   контейнер — `0`. Content-related требуют подтверждения (ack).

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ProtocolError;

/// Допустимое «будущее» расхождение часов, секунды.
pub const FUTURE_SKEW_SECS: i64 = 30;
/// Допустимое «прошлое» расхождение часов, секунды.
pub const PAST_SKEW_SECS: i64 = 300;
/// Чётность клиентских `msg_id` (`% 4 == 0`).
pub const PARITY_CLIENT: i64 = 0;
/// Чётность серверных `msg_id` (`% 4 == 1`).
pub const PARITY_SERVER: i64 = 1;

const COUNTER_MASK: u32 = 0x3FFF_FFFF;

/// Текущее время в секундах (unix epoch).
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Время из `msg_id` (старшие 32 бита).
pub fn msg_id_time_secs(msg_id: i64) -> i64 {
    msg_id >> 32
}

/// Собирает `msg_id` из времени, счётчика и чётности.
pub fn make_msg_id(secs: i64, counter: u32, parity: i64) -> i64 {
    (secs << 32) | (((counter & COUNTER_MASK) as i64) << 2) | (parity & 0b11)
}

/// Генератор монотонных `msg_id` для одной стороны.
#[derive(Debug, Clone)]
pub struct MsgIdGenerator {
    last: Option<i64>,
    counter: u32,
    parity: i64,
}

impl MsgIdGenerator {
    /// Генератор клиентской стороны (`% 4 == 0`).
    pub fn client() -> Self {
        Self::new(PARITY_CLIENT)
    }

    /// Генератор серверной стороны (`% 4 == 1`).
    pub fn server() -> Self {
        Self::new(PARITY_SERVER)
    }

    /// Генератор с заданной чётностью.
    pub fn new(parity: i64) -> Self {
        MsgIdGenerator {
            last: None,
            counter: 0,
            parity,
        }
    }

    /// Выдаёт следующий `msg_id`, монотонный даже при откате часов назад.
    pub fn next(&mut self, now_secs: i64) -> i64 {
        let mut candidate = make_msg_id(now_secs, self.counter, self.parity);
        if let Some(last) = self.last {
            if candidate <= last {
                candidate = last + 4;
            }
        }
        self.counter = (self.counter + 1) & COUNTER_MASK;
        self.last = Some(candidate);
        candidate
    }

    /// Последний выданный `msg_id`.
    pub fn last(&self) -> Option<i64> {
        self.last
    }

    /// Текущее значение счётчика (для сохранения в БД).
    pub fn counter(&self) -> u32 {
        self.counter
    }

    /// Восстанавливает состояние после перезапуска.
    pub fn with_state(parity: i64, counter: u32) -> Self {
        MsgIdGenerator {
            last: None,
            counter: counter & COUNTER_MASK,
            parity,
        }
    }
}

/// Проверка входящих `msg_id` (чётность, окно времени, монотонность, дубли).
#[derive(Debug, Clone)]
pub struct MsgIdValidator {
    last_recv: Option<i64>,
    peer_parity: i64,
}

impl MsgIdValidator {
    /// Валидатор сообщений, присланных клиентом (`% 4 == 0`).
    pub fn client() -> Self {
        Self::new(PARITY_CLIENT)
    }

    /// Валидатор сообщений, присланных сервером (`% 4 == 1`).
    pub fn server() -> Self {
        Self::new(PARITY_SERVER)
    }

    /// Валидатор с заданной чётностью отправителя.
    pub fn new(peer_parity: i64) -> Self {
        MsgIdValidator {
            last_recv: None,
            peer_parity,
        }
    }

    /// Проверяет `msg_id` и запоминает его как последний принятый.
    ///
    /// Возвращает [`ProtocolError::Duplicate`] для повторного или
    /// не возрастающего `msg_id`: вызывающий код должен сначала поискать
    /// сохранённый ответ, а потом уже считать это ошибкой.
    pub fn validate(&mut self, msg_id: i64, now_secs: i64) -> Result<(), ProtocolError> {
        if msg_id & 0b11 != self.peer_parity {
            return Err(ProtocolError::MsgId(format!(
                "msg_id 0x{msg_id:016x} has wrong parity (expected {})",
                self.peer_parity
            )));
        }

        let msg_secs = msg_id_time_secs(msg_id);
        if msg_secs - now_secs > FUTURE_SKEW_SECS || now_secs - msg_secs > PAST_SKEW_SECS {
            return Err(ProtocolError::TimeSkew {
                msg_id_secs: msg_secs,
                server_secs: now_secs,
            });
        }

        if let Some(last) = self.last_recv {
            if msg_id <= last {
                return Err(ProtocolError::Duplicate);
            }
        }
        self.last_recv = Some(msg_id);
        Ok(())
    }

    /// Последний принятый `msg_id`.
    pub fn last(&self) -> Option<i64> {
        self.last_recv
    }

    /// Восстанавливает состояние после перезапуска.
    pub fn with_last(peer_parity: i64, last: i64) -> Self {
        MsgIdValidator {
            last_recv: if last > 0 { Some(last) } else { None },
            peer_parity,
        }
    }
}

/// Генератор `seq_no`: отдельные счётчики для content-related и сервисных.
#[derive(Debug, Clone, Default)]
pub struct SeqNoGenerator {
    content: u32,
    other: u32,
}

impl SeqNoGenerator {
    /// Новый генератор.
    pub fn new() -> Self {
        Self::default()
    }

    /// `seq_no` для content-related сообщения (`2n + 1`).
    pub fn next_content(&mut self) -> i32 {
        let value = 2 * self.content + 1;
        self.content = self.content.wrapping_add(1);
        value as i32
    }

    /// `seq_no` для сервисного сообщения (`2n`).
    pub fn next_non_content(&mut self) -> i32 {
        let value = 2 * self.other;
        self.other = self.other.wrapping_add(1);
        value as i32
    }

    /// Текущие счётчики `(content, other)` для сохранения в БД.
    pub fn state(&self) -> (u32, u32) {
        (self.content, self.other)
    }

    /// Восстанавливает счётчики после перезапуска.
    pub fn with_state(content: u32, other: u32) -> Self {
        SeqNoGenerator { content, other }
    }
}

/// Является ли `seq_no` content-related (нечётный, неотрицательный).
pub fn is_content_seq(seq_no: i32) -> bool {
    seq_no >= 0 && (seq_no & 1) == 1
}

/// Является ли `seq_no` сервисным (чётный, неотрицательный).
pub fn is_non_content_seq(seq_no: i32) -> bool {
    seq_no >= 0 && (seq_no & 1) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_are_even_and_monotonic() {
        let mut generator = MsgIdGenerator::client();
        let now = 1_800_000_000;
        let first = generator.next(now);
        let second = generator.next(now);
        assert_eq!(first & 0b11, 0);
        assert_eq!(second & 0b11, 0);
        assert!(second > first);
        assert_eq!(msg_id_time_secs(first), now);
    }

    #[test]
    fn server_ids_have_parity_one() {
        let mut generator = MsgIdGenerator::server();
        let id = generator.next(1_800_000_000);
        assert_eq!(id & 0b11, 1);
    }

    #[test]
    fn generator_stays_monotonic_when_clock_goes_back() {
        let mut generator = MsgIdGenerator::client();
        let first = generator.next(1_800_000_000);
        let second = generator.next(1_700_000_000);
        let third = generator.next(1_800_000_000);
        assert!(second > first);
        assert!(third > second);
        assert_eq!(second & 0b11, 0);
    }

    #[test]
    fn validator_accepts_fresh_messages_and_rejects_duplicates() {
        let mut validator = MsgIdValidator::client();
        let mut generator = MsgIdGenerator::client();
        let now = 1_800_000_000;
        let id = generator.next(now);

        validator.validate(id, now).unwrap();
        assert!(matches!(
            validator.validate(id, now),
            Err(ProtocolError::Duplicate)
        ));

        let next = generator.next(now);
        validator.validate(next, now).unwrap();
        assert_eq!(validator.last(), Some(next));
    }

    #[test]
    fn validator_rejects_wrong_parity_and_skew() {
        let now = 1_800_000_000;
        let mut validator = MsgIdValidator::client();

        let wrong_parity = make_msg_id(now, 0, PARITY_SERVER);
        assert!(matches!(
            validator.validate(wrong_parity, now),
            Err(ProtocolError::MsgId(_))
        ));

        let too_far_future = make_msg_id(now + FUTURE_SKEW_SECS + 10, 0, PARITY_CLIENT);
        assert!(matches!(
            validator.validate(too_far_future, now),
            Err(ProtocolError::TimeSkew { .. })
        ));

        let too_far_past = make_msg_id(now - PAST_SKEW_SECS - 10, 0, PARITY_CLIENT);
        assert!(matches!(
            validator.validate(too_far_past, now),
            Err(ProtocolError::TimeSkew { .. })
        ));
    }

    #[test]
    fn seq_no_numbering_follows_mtproto() {
        let mut generator = SeqNoGenerator::new();
        assert_eq!(generator.next_content(), 1);
        assert_eq!(generator.next_content(), 3);
        assert_eq!(generator.next_non_content(), 0);
        assert_eq!(generator.next_non_content(), 2);

        assert!(is_content_seq(1));
        assert!(is_content_seq(3));
        assert!(!is_content_seq(0));
        assert!(is_non_content_seq(0));
        assert!(is_non_content_seq(2));
        assert!(!is_content_seq(-1));
    }
}
