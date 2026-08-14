//! Этап 1: SIMD-поиск значимых байтов.
//!
//! Один проход по документу блоками 16/32 байта. Для каждого блока
//! загружается регистр, для каждого интересующего байта выполняется
//! `cmpeq`, маски объединяются через OR. Установленные биты маски —
//! позиции найденных байтов внутри регистра.
//!
//! SIMD-слой ничего не знает о синтаксисе Zoll — только «интересующие байты».
//! Результат — поток событий `(position, byte)`, отсортированный по позиции.

/// Событие первичного потока: найденный байт и его абсолютная позиция.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub position: usize,
    pub byte: u8,
}

/// Сканирует текст и возвращает позиции всех интересующих байтов.
///
/// Один проход, без потоков: SIMD умеет искать несколько символов за раз
/// (cmpeq по каждому таргету, маски OR). Потоки не нужны.
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
    scan_scalar(text, targets)
}

/// AVX2: блоки по 32 байта.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_avx2(text: &[u8], targets: &[u8]) -> Vec<Event> {
    use std::arch::x86_64::*;
    unsafe {
        let mut events = Vec::new();
        let len = text.len();
        let mut i = 0;

        while i + 32 <= len {
            let block = _mm256_loadu_si256(text.as_ptr().add(i) as *const __m256i);
            let mut mask = _mm256_setzero_si256();
            for &t in targets {
                let needle = _mm256_set1_epi8(t as i8);
                mask = _mm256_or_si256(mask, _mm256_cmpeq_epi8(block, needle));
            }
            let bits = _mm256_movemask_epi8(mask) as u32;
            let mut b = bits;
            while b != 0 {
                let bit = b.trailing_zeros() as usize;
                let pos = i + bit;
                events.push(Event {
                    position: pos,
                    byte: text[pos],
                });
                b &= b - 1;
            }
            i += 32;
        }

        // Хвост (меньше блока) — скалярно.
        for j in i..len {
            if targets.contains(&text[j]) {
                events.push(Event {
                    position: j,
                    byte: text[j],
                });
            }
        }
        events
    }
}

/// SSE2: блоки по 16 байт (базовый уровень x86-64).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_sse2(text: &[u8], targets: &[u8]) -> Vec<Event> {
    use std::arch::x86_64::*;
    unsafe {
        let mut events = Vec::new();
        let len = text.len();
        let mut i = 0;

        while i + 16 <= len {
            let block = _mm_loadu_si128(text.as_ptr().add(i) as *const __m128i);
            let mut mask = _mm_setzero_si128();
            for &t in targets {
                let needle = _mm_set1_epi8(t as i8);
                mask = _mm_or_si128(mask, _mm_cmpeq_epi8(block, needle));
            }
            let bits = _mm_movemask_epi8(mask) as u32;
            let mut b = bits;
            while b != 0 {
                let bit = b.trailing_zeros() as usize;
                let pos = i + bit;
                events.push(Event {
                    position: pos,
                    byte: text[pos],
                });
                b &= b - 1;
            }
            i += 16;
        }

        for j in i..len {
            if targets.contains(&text[j]) {
                events.push(Event {
                    position: j,
                    byte: text[j],
                });
            }
        }
        events
    }
}

/// Скалярный фолбэк (не-x86 или без SIMD).
fn scan_scalar(text: &[u8], targets: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    for (i, &b) in text.iter().enumerate() {
        if targets.contains(&b) {
            events.push(Event {
                position: i,
                byte: b,
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
        let positions: Vec<usize> = events.iter().map(|e| e.position).collect();
        assert_eq!(positions, vec![1, 3, 4, 6, 7]);
        assert_eq!(events[0].byte, b'*');
        assert_eq!(events[3].byte, b')');
    }

    #[test]
    fn finds_newlines() {
        let text = b"line1\nline2\n";
        let events = scan(text, b"\n");
        let positions: Vec<usize> = events.iter().map(|e| e.position).collect();
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
        for w in events.windows(2) {
            assert!(w[0].position < w[1].position);
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
