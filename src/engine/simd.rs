//! Этап 1: SIMD-поиск значимых байтов.
//!
//! Один проход по документу блоками 16/32 байта. Для каждого блока
//! загружается регистр и вычисляется битовая маска позиций интересующих
//! байтов. Установленные биты маски — позиции найденных байтов внутри блока.
//!
//! Результат отдаётся сразу, без промежуточных векторов: на каждый блок
//! вызывается `emit(offset, mask)`, где бит `i` установлен, если
//! `text[offset + i]` — один из интересующих байтов. Размер блока известен
//! заранее (32 байта AVX2 / 16 байт SSE2/NEON), поэтому вектор не нужен.
//!
//! SIMD-слой не знает о синтаксисе Zoll: он только находит интересующие
//! байты и отдаёт маски. Какой байт находится по установленному биту,
//! решает потребитель (`text[offset + bit]`).
//!
//! ## Два способа поиска
//!
//! 1. **Табличный** (`pshufb`/`vtbl1q_u8`) — основной путь. Вместо `cmpeq`
//!    на каждый целевой байт строится пара таблиц `E` и `BIT` (см.
//!    `build_tables`), и каждый байт классифицируется по своим двум нибблам
//!    за пару подстановок + AND. Точный, без ложных срабатываний. Требует
//!    `pshufb` (SSSE3/AVX2) или `vtbl1q_u8` (NEON).
//! 2. **`cmpeq`** — фолбэк для SSE2 (нет `pshufb`) и для наборов целей с
//!    байтами ≥ `0x80` (старший ниббл 8–15 не влезает в 8-битную маску `E`).

// Сканирует текст и передаёт битовую маску каждого блока в `emit`.
//
// Маска имеет фиксированный размер: `u32` на 32-байтный блок AVX2,
// младшие 16 бит используются на 16-байтных блоках SSE2/NEON/скаляра.
// Нулевые маски (в блоке нет интересующих байтов) не передаются.
//
// Выбирает табличный путь (`pshufb`/`vtbl1q_u8`), если архитектура его
// поддерживает и все целевые байты < `0x80`; иначе — `cmpeq`-фолбэк.
pub fn scan<F: FnMut(usize, u32)>(text: &[u8], targets: &[u8], emit: F) {
    let tables = build_tables(targets);
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            if let Some(t) = tables {
                return unsafe { scan_avx2_table(text, t, emit) };
            }
            return unsafe { scan_avx2(text, targets, emit) };
        }
        if std::arch::is_x86_feature_detected!("ssse3") {
            if let Some(t) = tables {
                return unsafe { scan_ssse3_table(text, t, emit) };
            }
            return unsafe { scan_sse2(text, targets, emit) };
        }
        if std::arch::is_x86_feature_detected!("sse2") {
            return unsafe { scan_sse2(text, targets, emit) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON — базовый уровень AArch64, всегда доступен.
        if let Some(t) = tables {
            return unsafe { scan_neon_table(text, t, emit) };
        }
        return unsafe { scan_neon(text, targets, emit) };
    }
    scan_scalar(text, targets, emit);
}

// Строит таблицы `E` и `BIT` для табличного поиска.
//
// `E[n]` — битовая маска валидных старших нибблов для младшего ниббла `n`
// (бит `h` установлен ⟺ байт `(h << 4) | n` — цель). `BIT[n] = 1 << n`.
//
// Байт `b = (h << 4) | n` — цель ⟺ `E[n] & (1 << h) != 0`. Это точный тест
// (без ложных срабатываний), т.к. проверяется конкретная пара нибблов.
//
// Возвращает `None`, если любой целевой байт ≥ `0x80`: старший ниббл 8–15
// не влезает в 8-битную маску `E`. Тогда вызывающий использует `cmpeq`.
fn build_tables(targets: &[u8]) -> Option<([u8; 16], [u8; 16])> {
    let mut e = [0u8; 16];
    for &t in targets {
        if t >= 0x80 {
            return None;
        }
        let hi = (t >> 4) as usize;
        let lo = (t & 0x0F) as usize;
        e[lo] |= 1 << hi;
    }
    // BIT[n] = 1 << n. Нужны только n 0..8 (старшие нибблы целей < 8).
    let mut bit = [0u8; 16];
    for i in 0..8 {
        bit[i] = 1 << i;
    }
    Some((e, bit))
}

// AVX2: табличный путь, блоки по 32 байта.
//
// Старший ниббл извлекается через even/odd split (в AVX2 нет побайтового
// сдвига): `srli_epi16` сдвигает 16-битные пары и портит нечётные байты,
// поэтому нечётные байты сдвигаются отдельно (вектор на 1 байт влево) и
// результаты склеиваются через `blendv`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_avx2_table<F: FnMut(usize, u32)>(
    text: &[u8],
    tables: ([u8; 16], [u8; 16]),
    mut emit: F,
) {
    use std::arch::x86_64::*;
    unsafe {
        let (e_table, bit_table) = tables;
        let e_vec =
            _mm256_broadcastsi128_si256(_mm_loadu_si128(e_table.as_ptr() as *const __m128i));
        let bit_vec =
            _mm256_broadcastsi128_si256(_mm_loadu_si128(bit_table.as_ptr() as *const __m128i));
        // 0xFF на чётных позициях (0,2,...,30), 0 на нечётных.
        let even_mask = _mm256_set_epi8(
            0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0,
            -1, 0, -1, 0, -1, 0, -1,
        );
        let zero = _mm256_setzero_si256();
        let ones = _mm256_set1_epi8(-1);

        let len = text.len();
        let mut offset = 0;

        while offset + 32 <= len {
            let v = _mm256_loadu_si256(text.as_ptr().add(offset) as *const __m256i);

            // Извлечение старшего ниббла каждого байта.
            // srli_epi16(x,4) даёт верный старший ниббл на НЕчётных позициях
            // (чётные портятся). Поэтому чётные байты сдвигаем в нечётные
            // позиции (slli_si256 на 1), извлекаем и сдвигаем обратно.
            let v_left = _mm256_slli_si256::<1>(v);
            let hi_even = _mm256_srli_epi16::<4>(v_left);
            let hi_even = _mm256_srli_si256::<1>(hi_even);
            let hi_odd = _mm256_srli_epi16::<4>(v);
            let hi = _mm256_blendv_epi8(hi_odd, hi_even, even_mask);

            // Классификация: E[младший ниббл] & (1 << старший ниббл).
            // shuffle_epi8(данные, индексы): данные = таблица, индексы = байты.
            let code = _mm256_shuffle_epi8(e_vec, v);
            let bit = _mm256_shuffle_epi8(bit_vec, hi);
            let matchv = _mm256_and_si256(code, bit);
            // Ненулевой → 0xFF.
            let matchv = _mm256_xor_si256(_mm256_cmpeq_epi8(matchv, zero), ones);
            let bits = _mm256_movemask_epi8(matchv) as u32;
            if bits != 0 {
                emit(offset, bits);
            }
            offset += 32;
        }

        // Остаток (меньше блока) — скалярно, той же таблицей.
        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            let hi = (byte >> 4) as usize;
            if byte < 0x80 && (e_table[(byte & 0x0F) as usize] & (1u8 << hi)) != 0 {
                remainder_mask |= 1 << rel;
            }
        }
        if remainder_mask != 0 {
            emit(offset, remainder_mask);
        }
    }
}

// AVX2: cmpeq-фолбэк, блоки по 32 байта.
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

// SSSE3: табличный путь, блоки по 16 байт.
//
// То же, что AVX2, но на 128-бит; blend через and/or (в SSSE3 нет `blendv`).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn scan_ssse3_table<F: FnMut(usize, u32)>(
    text: &[u8],
    tables: ([u8; 16], [u8; 16]),
    mut emit: F,
) {
    use std::arch::x86_64::*;
    unsafe {
        let (e_table, bit_table) = tables;
        let e_vec = _mm_loadu_si128(e_table.as_ptr() as *const __m128i);
        let bit_vec = _mm_loadu_si128(bit_table.as_ptr() as *const __m128i);
        let even_mask = _mm_set_epi8(0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1);
        let odd_mask = _mm_set_epi8(-1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0, -1, 0);
        let zero = _mm_setzero_si128();
        let ones = _mm_set1_epi8(-1);

        let len = text.len();
        let mut offset = 0;

        while offset + 16 <= len {
            let v = _mm_loadu_si128(text.as_ptr().add(offset) as *const __m128i);

            // Извлечение старшего ниббла каждого байта.
            // srli_epi16(x,4) даёт верный старший ниббл на НЕчётных позициях
            // (чётные портятся). Поэтому чётные байты сдвигаем в нечётные
            // позиции (slli_si128 на 1), извлекаем и сдвигаем обратно.
            let v_left = _mm_slli_si128::<1>(v);
            let hi_even = _mm_srli_epi16::<4>(v_left);
            let hi_even = _mm_srli_si128::<1>(hi_even);
            let hi_odd = _mm_srli_epi16::<4>(v);
            let hi = _mm_or_si128(
                _mm_and_si128(hi_even, even_mask),
                _mm_and_si128(hi_odd, odd_mask),
            );

            // Классификация: E[младший ниббл] & (1 << старший ниббл).
            // shuffle_epi8(данные, индексы): данные = таблица, индексы = байты.
            let code = _mm_shuffle_epi8(e_vec, v);
            let bit = _mm_shuffle_epi8(bit_vec, hi);
            let matchv = _mm_and_si128(code, bit);
            let matchv = _mm_xor_si128(_mm_cmpeq_epi8(matchv, zero), ones);
            let bits = _mm_movemask_epi8(matchv) as u32;
            if bits != 0 {
                emit(offset, bits);
            }
            offset += 16;
        }

        // Остаток (меньше блока) — скалярно, той же таблицей.
        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            let hi = (byte >> 4) as usize;
            if byte < 0x80 && (e_table[(byte & 0x0F) as usize] & (1u8 << hi)) != 0 {
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

// NEON (aarch64): табличный путь, блоки по 16 байт.
//
// В NEON есть побайтовый сдвиг `vshrq_n_u8`, поэтому старший ниббл
// извлекается одной операцией — без even/odd split.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn scan_neon_table<F: FnMut(usize, u32)>(
    text: &[u8],
    tables: ([u8; 16], [u8; 16]),
    mut emit: F,
) {
    use std::arch::aarch64::*;
    unsafe {
        let (e_table, bit_table) = tables;
        let e_vec = vld1q_u8(e_table.as_ptr());
        let bit_vec = vld1q_u8(bit_table.as_ptr());
        let zero = vdupq_n_u8(0);

        let len = text.len();
        let mut offset = 0;

        while offset + 16 <= len {
            let v = vld1q_u8(text.as_ptr().add(offset));

            // Старший ниббл каждого байта — побайтовый сдвиг вправо на 4.
            let hi = vshrq_n_u8(v, 4);
            // Классификация: E[младший ниббл] & (1 << старший ниббл).
            let code = vqtbl1q_u8(e_vec, v);
            let bit = vqtbl1q_u8(bit_vec, hi);
            let matchv = vandq_u8(code, bit);
            // Ненулевой → 0xFF (unsigned > 0).
            let matchv = vcgtq_u8(matchv, zero);
            // Собираем 16 бит из двух u64-лан.
            let lo = vgetq_lane_u64(vreinterpretq_u64_u8(matchv), 0);
            let hi64 = vgetq_lane_u64(vreinterpretq_u64_u8(matchv), 1);
            let bits = (lo | (hi64 << 8)) as u32;
            if bits != 0 {
                emit(offset, bits);
            }
            offset += 16;
        }

        // Остаток (меньше блока) — скалярно, той же таблицей.
        let mut remainder_mask = 0u32;
        for (rel, &byte) in text[offset..].iter().enumerate() {
            let hi = (byte >> 4) as usize;
            if byte < 0x80 && (e_table[(byte & 0x0F) as usize] & (1u8 << hi)) != 0 {
                remainder_mask |= 1 << rel;
            }
        }
        if remainder_mask != 0 {
            emit(offset, remainder_mask);
        }
    }
}

// NEON (aarch64): cmpeq-фолбэк, блоки по 16 байт.
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

    #[test]
    fn table_no_false_positives() {
        // Цифры, буквы, управляющие и байты ≥ 0x80 — не должны давать
        // ложных срабатываний (табличный путь vs скаляр).
        let text = b"abc123XYZ 456 \x00\x01\x7f\x80\xff !#*";
        let targets = b"*/_~=+-',$%!#>|:.)}\n";
        let simd_events = scan_events(text, targets);
        let scalar_events = scan_events_scalar(text, targets);
        assert_eq!(simd_events, scalar_events);
    }

    #[test]
    fn table_fallback_high_bytes() {
        // Цель ≥ 0x80 → build_tables возвращает None → cmpeq-фолбэк.
        let text = b"a\x80b\x81c\x80";
        let targets = b"\x80\x81";
        let simd_events = scan_events(text, targets);
        let scalar_events = scan_events_scalar(text, targets);
        assert_eq!(simd_events, scalar_events);
    }

    #[test]
    fn table_matches_all_targets() {
        // Каждый целевой байт обязан находиться табличным путём.
        let targets = b"*/_~=+-',$%!#>|:.)}\n";
        let text: Vec<u8> = targets.to_vec();
        let simd_events = scan_events(&text, targets);
        let scalar_events = scan_events_scalar(&text, targets);
        assert_eq!(simd_events, scalar_events);
        assert_eq!(simd_events.len(), targets.len());
    }

    #[test]
    fn table_randomized_matches_scalar() {
        // Детерминированный генератор (без rand): табличный путь обязан
        // совпадать со скаляром на любых случайных входах и наборах целей.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        };
        let target_sets: &[&[u8]] = &[
            b"*/_~=+-',$%!#>|:.)}\n",
            b"abc",
            b"\x00\x7f",
            b"",
            b"*",
            b"\x01\x02\x03\x04",
        ];
        for &targets in target_sets {
            for _ in 0..300 {
                let len = (next() as usize) % 100;
                let text: Vec<u8> = (0..len).map(|_| next()).collect();
                let simd = scan_events(&text, targets);
                let scalar = scan_events_scalar(&text, targets);
                assert_eq!(simd, scalar, "targets={targets:?} text={text:?}");
            }
        }
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
