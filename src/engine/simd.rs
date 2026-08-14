//! Этап 1: SIMD-поиск значимых байтов.
//!
//! Один проход по документу блоками 16/32 байта. Для каждого блока
//! загружается регистр, для каждого интересующего байта выполняется
//! `cmpeq`, маски объединяются через OR. Установленные биты маски —
//! позиции найденных байтов внутри блока.
//!
//! Результат отдаётся сразу, без промежуточных векторов: на каждый блок
//! вызывается `emit(offset, mask)`, где бит `i` установлен, если
//! `text[offset + i]` — один из интересующих байтов. Размер блока известен
//! заранее (32 байта AVX2 / 16 байт SSE2/NEON), поэтому вектор не нужен.
//!
//! SIMD-слой не знает о синтаксисе Zoll: он только находит интересующие
//! байты и отдаёт маски. Какой байт находится по установленному биту,
//! решает потребитель (`text[offset + bit]`).

// Сканирует текст и передаёт битовую маску каждого блока в `emit`.
//
// Маска имеет фиксированный размер: `u32` на 32-байтный блок AVX2,
// младшие 16 бит используются на 16-байтных блоках SSE2/NEON/скаляра.
// Нулевые маски (в блоке нет интересующих байтов) не передаются.
pub fn scan<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], emit: F) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { scan_avx2(text, targets, emit) };
        }
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { scan_sse2(text, targets, emit) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON — базовый уровень AArch64, всегда доступен.
        return unsafe { scan_neon(text, targets, emit) };
    }
    scan_scalar(text, targets, emit);
}

// AVX2: блоки по 32 байта.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_avx2<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], mut emit: F) {
    use std::arch::x86_64::*;
    unsafe {
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
            if bits != 0 {
                emit(offset, bits);
            }
            offset += 32;
        }

        // Остаток (меньше блока) — скалярно, той же маской.
        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                remainder_mask |= 1 << rel;
            }
        }
        if remainder_mask != 0 {
            emit(offset, remainder_mask);
        }
    }
}

// SSE2: блоки по 16 байт (базовый уровень x86-64).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_sse2<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], mut emit: F) {
    use std::arch::x86_64::*;
    unsafe {
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
            if bits != 0 {
                emit(offset, bits);
            }
            offset += 16;
        }

        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                remainder_mask |= 1 << rel;
            }
        }
        if remainder_mask != 0 {
            emit(offset, remainder_mask);
        }
    }
}

// NEON (aarch64): блоки по 16 байт.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn scan_neon<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], mut emit: F) {
    use std::arch::aarch64::*;
    unsafe {
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
            if bits != 0 {
                emit(offset, bits as u32);
            }
            offset += 16;
        }

        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            if targets.contains(&byte) {
                remainder_mask |= 1 << rel;
            }
        }
        if remainder_mask != 0 {
            emit(offset, remainder_mask);
        }
    }
}

// Скалярный фолбэк (не-x86 или без SIMD): блоки по 16 байт.
fn scan_scalar<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], mut emit: F) {
    let len = text.len();
    let mut offset = 0;

    while offset + 16 <= len {
        let mut mask = 0u32;
        for rel in 0..16 {
            if targets.contains(&text[offset + rel]) {
                mask |= 1 << rel;
            }
        }
        if mask != 0 {
            emit(offset, mask);
        }
        offset += 16;
    }

    let mut remainder_mask = 0u32;
    for (rel, &byte) in text[offset..].iter().enumerate() {
        if targets.contains(&byte) {
            remainder_mask |= 1 << rel;
        }
    }
    if remainder_mask != 0 {
        emit(offset, remainder_mask);
    }
}

// Тестовая обёртка: собирает события `(позиция, байт)` из масок.
// Используется только в тестах.
#[cfg(test)]
fn scan_events(text: &[u8], targets: &[u8]) -> Vec<(usize, u8)> {
    let mut events = Vec::new();
    scan(text, targets, |offset, mask| {
        let mut remaining = mask;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            events.push((offset + bit, text[offset + bit]));
            remaining &= remaining - 1;
        }
    });
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_interesting_bytes() {
        let text = b"a*b**c))d";
        let events = scan_events(text, b"*)");
        let positions: Vec<usize> = events.iter().map(|event| event.0).collect();
        assert_eq!(positions, vec![1, 3, 4, 6, 7]);
        assert_eq!(events[0].1, b'*');
        assert_eq!(events[3].1, b')');
    }

    #[test]
    fn finds_newlines() {
        let text = b"line1\nline2\n";
        let events = scan_events(text, b"\n");
        let positions: Vec<usize> = events.iter().map(|event| event.0).collect();
        assert_eq!(positions, vec![5, 11]);
    }

    #[test]
    fn empty_input() {
        assert!(scan_events(b"", b"*").is_empty());
    }

    #[test]
    fn no_matches() {
        assert!(scan_events(b"plain text", b"*").is_empty());
    }

    #[test]
    fn events_sorted() {
        let text = b"x*y)z*w";
        let events = scan_events(text, b"*)");
        for pair in events.windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
    }

    #[test]
    fn scalar_matches_simd() {
        let text = "**//текст)) #1# Заголовок\n%% комментарий".as_bytes();
        let targets = b"*/_~=+-',$%!#>|:.)}\n";
        let mut simd_events = Vec::new();
        scan(text, targets, |offset, mask| {
            let mut remaining = mask;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                simd_events.push((offset + bit, text[offset + bit]));
                remaining &= remaining - 1;
            }
        });
        let scalar_events = scan_events_scalar(text, targets);
        assert_eq!(simd_events, scalar_events);
    }

    // Скалярная версия без SIMD-диспетчера (для сверки).
    fn scan_events_scalar(text: &[u8], targets: &[u8]) -> Vec<(usize, u8)> {
        let mut events = Vec::new();
        scan_scalar(text, targets, |offset, mask| {
            let mut remaining = mask;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                events.push((offset + bit, text[offset + bit]));
                remaining &= remaining - 1;
            }
        });
        events
    }

    #[test]
    fn masks_cover_tail() {
        // 32 байта полного блока + остаток: `*` обязан попасть в маску остатка.
        let text = b"12345678901234567890123456789012*"; // 32 байта + 1 остаток
        let events = scan_events(text, b"*");
        assert_eq!(events, vec![(32, b'*')]);
    }
}
