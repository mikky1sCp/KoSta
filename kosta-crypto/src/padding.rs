use rand::Rng;
use rand::rngs::OsRng;

/// Дополняет данные случайными байтами так, чтобы итоговая длина стала кратна 16.
/// Добавляется от 12 до 1024 байт (с коррекцией до кратности).
pub fn pad(data: &[u8]) -> Vec<u8> {
    let mut rng = OsRng;
    let current_len = data.len();
    // минимально необходимое дополнение до кратности 16
    let remainder = (16 - (current_len % 16)) % 16;
    let min_pad = core::cmp::max(12, remainder);
    // случайный размер дополнения от min_pad до 1024
    let pad_len = rng.gen_range(min_pad..=1024);
    // скорректируем, чтобы итоговая длина стала кратна 16
    let total_pad = pad_len + (16 - ((current_len + pad_len) % 16)) % 16;
    let mut padded = Vec::with_capacity(current_len + total_pad);
    padded.extend_from_slice(data);
    padded.extend((0..total_pad).map(|_| rng.gen::<u8>()));
    padded
}

/// Извлекает исходные данные, отбрасывая padding (оригинальная длина известна вызывающему).
pub fn unpad(padded: &[u8], original_len: usize) -> &[u8] {
    &padded[..original_len]
}