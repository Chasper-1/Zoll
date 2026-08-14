//! Этап 1: SIMD-поиск значимых байтов.
//!
//! Один проход по документу блоками 16/32 байта. Для каждого блока
//! загружается регистр, для каждого интересующего байта выполняется
//! `cmpeq`, маски объединяются через OR. Установленные биты маски —
//! позиции найденных байтов внутри регистра.
//!
//! SIMD-слой ничего не знает о синтаксисе Zoll — только «интересующие байты».
//! Результат — поток событий `(position, byte)`, отсортированный по позиции.

// Событие первичного потока: найденный байт и его абсолютная позиция.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub position: usize,
    pub byte: u8,
}

// Сканирует текст и возвращает позиции всех интересующих байтов.
//
// Один проход, без потоков: SIMD умеет искать несколько символов за раз
// (cmpeq по каждому таргету, маски OR). Потоки не нужны.
pub fn scan(text: &[u8], targets: &[u8]) -> Vec<Event> {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { scan_avx2(text, targets) };
        }
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { scan_sse2(text, targets) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON — базовый уровень AArch64, всегда доступен.
        return unsafe { scan_neon(text, targets) };
    }
    scan_scalar(text, targets)
}

// AVX2: блоки по 32 байта.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_avx2(text: &[u8], targets: &[u8]) -> Vec<Event> {
    use std::arch::x86_64::*;
    unsafe {
        let mut events = Vec::new();
        let len = text.len();
        let mut offset = 0;

        while offset + 32 <= len {
            let block = _mm256_loadu_si256(text.as_ptr().add(offset) as *const __m256i);
            let mut mask = _mm256_setzero_si256();
            for &target in targets {
                let needle = _mm256_set1_epi8(target as i8);
                mask = _mm256_or_si256(mask, _mm256_cmpeq_epi8(block, needle));
            }
            let bits = _mm256_movemask_epi8(mask) as u32;
            let mut remaining = bits;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                let pos = offset + bit;
                events.push(Event {
                    position: pos,
                    byte: text[pos],
                });
                remaining &= remaining - 1;
            }
            offset += 32;
        }

        // Хвост (меньше блока) — скалярно.
        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                events.push(Event {
                    position: offset + rel,
                    byte,
                });
            }
        }
        events
    }
}

// SSE2: блоки по 16 байт (базовый уровень x86-64).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_sse2(text: &[u8], targets: &[u8]) -> Vec<Event> {
    use std::arch::x86_64::*;
    unsafe {
        let mut events = Vec::new();
        let len = text.len();
        let mut offset = 0;

        while offset + 16 <= len {
            let block = _mm_loadu_si128(text.as_ptr().add(offset) as *const __m128i);
            let mut mask = _mm_setzero_si128();
            for &target in targets {
                let needle = _mm_set1_epi8(target as i8);
                mask = _mm_or_si128(mask, _mm_cmpeq_epi8(block, needle));
            }
            let bits = _mm_movemask_epi8(mask) as u32;
            let mut remaining = bits;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                let pos = offset + bit;
                events.push(Event {
                    position: pos,
                    byte: text[pos],
                });
                remaining &= remaining - 1;
            }
            offset += 16;
        }

        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                events.push(Event {
                    position: offset + rel,
                    byte,
                });
            }
        }
        events
    }
}

// NEON (aarch64): блоки по 16 байт.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn scan_neon(text: &[u8], targets: &[u8]) -> Vec<Event> {
    use std::arch::aarch64::*;
    unsafe {
        let mut events = Vec::new();
        let len = text.len();
        let mut offset = 0;

        while offset + 16 <= len {
            let block = vld1q_u8(text.as_ptr().add(offset));
            let mut mask = vdupq_n_u8(0);
            for &target in targets {
                let needle = vdupq_n_u8(target);
                let eq = vceqq_u8(block, needle);
                mask = vorrq_u8(mask, eq);
            }
            // mask: 0xFF на совпадении, 0x00 иначе. Собираем 16 бит из двух u64-лан.
            let lo = vgetq_lane_u64(vreinterpretq_u64_u8(mask), 0);
            let hi = vgetq_lane_u64(vreinterpretq_u64_u8(mask), 1);
            let bits = lo | (hi << 8);
            let mut remaining = bits;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                let pos = offset + bit;
                events.push(Event {
                    position: pos,
                    byte: text[pos],
                });
                remaining &= remaining - 1;
            }
            offset += 16;
        }

        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                events.push(Event {
                    position: offset + rel,
                    byte,
                });
            }
        }
        events
    }
}

// Скалярный фолбэк (не-x86 или без SIMD).
fn scan_scalar(text: &[u8], targets: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    for (pos, &byte) in text.iter().enumerate() {
        if targets.contains(&byte) {
            events.push(Event {
                position: pos,
                byte,
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_interesting_bytes() {
        let text = b"a*b**c))d";
        let events = scan(text, b"*)");
        let positions: Vec<usize> = events.iter().map(|event| event.position).collect();
        assert_eq!(positions, vec![1, 3, 4, 6, 7]);
        assert_eq!(events[0].byte, b'*');
        assert_eq!(events[3].byte, b')');
    }

    #[test]
    fn finds_newlines() {
        let text = b"line1\nline2\n";
        let events = scan(text, b"\n");
        let positions: Vec<usize> = events.iter().map(|event| event.position).collect();
        assert_eq!(positions, vec![5, 11]);
    }

    #[test]
    fn empty_input() {
        assert!(scan(b"", b"*").is_empty());
    }

    #[test]
    fn no_matches() {
        assert!(scan(b"plain text", b"*").is_empty());
    }

    #[test]
    fn events_sorted() {
        let text = b"x*y)z*w";
        let events = scan(text, b"*)");
        for pair in events.windows(2) {
            assert!(pair[0].position < pair[1].position);
        }
    }

    #[test]
    fn scalar_matches_simd() {
        let text = "**//текст)) #1# Заголовок\n%% комментарий".as_bytes();
        let targets = b"*/_~=+-',$%!#>|:)}\n";
        let simd = scan(text, targets);
        let scalar = scan_scalar(text, targets);
        assert_eq!(simd, scalar);
    }
}
