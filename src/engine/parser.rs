//! Движок-парсер: публичный интерфейс `Engine`.
//!
//! Пайплайн:
//!
//! ```text
//! байты документа
//!     ↓
//! SIMD-скан → битовые маски блоков   (simd::scan)
//!     ↓
//! этап 1: регистры → готовые строки (карта `\n`)
//!     ↓
//! этап 2: грамматика на готовых строках (resolver::process_marker)
//!     ↓
//! синтаксические диапазоны (SyntaxSpan)
//! ```
//!
//! Единая координатная система — абсолютная позиция в байтах.
//! Редактор получает исходный буфер и диапазоны в byte offsets.

use std::borrow::Cow;

use crate::engine::api::{SpanSink, dispatch_spans};
use crate::engine::dependency::DependencyGraph;
use crate::engine::edit::Edit;
use crate::engine::line_map::LineMap;
use crate::engine::resolver::{ResolveState, SyntaxSpan, process_marker};
use crate::engine::simd::scan;

// Набор интересующих байтов: синтаксические + структурный `\n`.
pub const INTERESTING_BYTES: &[u8] = b"*/_~=+-',$%!#>|:.)}@\n";

// Движок парсера.
#[derive(Debug, Clone)]
pub struct Engine<'a> {
    // Исходный буфер документа: ссылка на буфер редактора (движок его
    // не меняет); при первой правке копируется (copy-on-write).
    pub text: Cow<'a, [u8]>,
    // Номер версии документа (раздел 17 спеки).
    pub revision: u64,
    // Карта строк.
    pub line_map: LineMap,
    // Синтаксические диапазоны.
    pub spans: Vec<SyntaxSpan>,
    // Граф зависимостей спанов.
    pub dependencies: DependencyGraph,
}

impl<'a> Engine<'a> {
    // Разобрать документ целиком.
    pub fn parse(text: &'a [u8]) -> Self {
        let (newline_positions, spans) = parse_document(text);
        let dependencies = DependencyGraph::new(&spans);
        Engine {
            text: Cow::Borrowed(text),
            revision: 0,
            line_map: LineMap::new(newline_positions),
            spans,
            dependencies,
        }
    }

    // Разобрать документ и сразу разослать спаны по ручке (fire-and-forget).
    pub fn parse_into(text: &'a [u8], sink: &mut dyn SpanSink) -> Self {
        let engine = Self::parse(text);
        dispatch_spans(sink, engine.revision, &engine.spans);
        engine
    }

    // Вставить текст в позицию и пересобрать спаны.
    //
    // Инкремент revision на каждую правку. Сейчас пересобирается весь
    // документ; оптимизация «только затронутые блоки» — следующий шаг.
    pub fn insert(&mut self, position: usize, bytes: &[u8]) -> &[SyntaxSpan] {
        self.apply_edit(&Edit::new(position, 0, bytes))
    }

    // Удалить кусок `[position, position + len)` и пересобрать спаны.
    pub fn delete(&mut self, position: usize, len: usize) -> &[SyntaxSpan] {
        self.apply_edit(&Edit::new(position, len, b""))
    }

    // Заменить кусок `[position, position + len)` на `bytes` и пересобрать
    // спаны.
    pub fn replace(&mut self, position: usize, len: usize, bytes: &[u8]) -> &[SyntaxSpan] {
        self.apply_edit(&Edit::new(position, len, bytes))
    }

    // Номер строки по байтовой позиции.
    pub fn line_at(&self, byte: usize) -> usize {
        self.line_map.line_at(byte)
    }

    // Внутренний путь: применить правку к буферу и пересобрать спаны.
    fn apply_edit(&mut self, edit: &Edit) -> &[SyntaxSpan] {
        // При первом редактировании заимствованный буфер копируется —
        // буфер редактора движок не меняет.
        edit.apply(self.text.to_mut());
        self.revision += 1;

        let (newline_positions, spans) = parse_document(self.text.as_ref());
        self.line_map = LineMap::new(newline_positions);
        self.spans = spans;
        self.dependencies = DependencyGraph::new(&self.spans);
        &self.spans
    }
}

// Парсинг в два этапа.
//
// Этап 1: регистры SIMD → готовые строки. Ищутся только `\n` — это карта
// строк. Никакой логики языка.
//
// Этап 2: грамматика на готовых строках. Второй проход по маркерам: у
// каждого маркера границы его строки (`line_start`, `line_end`) известны из
// карты, поэтому line-маркерам не нужно ничего искать.
//
// Возвращает `(позиции \n, синтаксические диапазоны)`.
pub(crate) fn parse_document(text: &[u8]) -> (Vec<usize>, Vec<SyntaxSpan>) {
    // ─── Этап 1: регистры SIMD → готовые строки ───
    let mut newline_positions: Vec<usize> = Vec::new();
    scan(text, b"\n", |offset, mask| {
        let mut remaining = mask;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            newline_positions.push(offset + bit);
            remaining &= remaining - 1;
        }
    });

    // ─── Этап 2: грамматика на готовых строках ───
    let mut state = ResolveState::new(text);
    state.line_end = newline_positions.first().copied().unwrap_or(text.len());
    let mut line_index = 0usize;

    // Текущий маркер: run подряд идущих одинаковых байтов.
    let mut marker_byte: u8 = 0;
    let mut marker_start: usize = 0;
    let mut marker_len: usize = 0;

    scan(text, INTERESTING_BYTES, |offset, mask| {
        let mut remaining = mask;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let pos = offset + bit;
            let byte = text[pos];
            remaining &= remaining - 1;

            // Продолжение текущего маркера.
            if byte == marker_byte && pos == marker_start + marker_len {
                marker_len += 1;
                continue;
            }
            // Завершаем предыдущий маркер и разбираем его.
            if marker_len > 0 {
                process_marker(&mut state, marker_byte, marker_start, marker_len);
                marker_len = 0;
            }
            if byte == b'\n' {
                // Строка кончилась — берём следующую из готовой карты.
                line_index += 1;
                state.line_start = pos + 1;
                state.line_end = newline_positions
                    .get(line_index)
                    .copied()
                    .unwrap_or(text.len());
                // Inline не выходит за строку — сбрасываем.
                state.inline_stack.clear();
            } else {
                marker_byte = byte;
                marker_start = pos;
                marker_len = 1;
            }
        }
    });
    // Последний маркер документа.
    if marker_len > 0 {
        process_marker(&mut state, marker_byte, marker_start, marker_len);
    }
    (newline_positions, state.spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SyntaxKind;

    #[test]
    fn parse_basic() {
        let engine = Engine::parse("**жирный))".as_bytes());
        assert_eq!(engine.spans.len(), 1);
        assert_eq!(engine.spans[0].kind, SyntaxKind::Bold);
        assert_eq!(engine.revision, 0);
    }

    #[test]
    fn edit_bumps_revision() {
        let mut engine = Engine::parse(b"hello");
        engine.insert(5, b" world");
        assert_eq!(engine.revision, 1);
        assert_eq!(engine.text.as_ref(), &b"hello world"[..]);
    }

    #[test]
    fn edit_creates_new_spans() {
        let mut engine = Engine::parse(b"plain");
        assert!(engine.spans.is_empty());
        engine.insert(0, b"**bold))");
        assert_eq!(engine.spans.len(), 1);
        assert_eq!(engine.spans[0].kind, SyntaxKind::Bold);
    }

    #[test]
    fn line_at_after_edit() {
        let mut engine = Engine::parse(b"a\nb");
        engine.insert(2, b"c\n");
        assert_eq!(engine.line_at(3), 1);
    }

    #[test]
    fn dependencies_tracked() {
        let engine = Engine::parse(b"**a)) **b))");
        assert_eq!(engine.dependencies.len(), 2);
    }

    #[test]
    fn line_map_built_in_one_pass() {
        // Карта строк строится в том же проходе, что и маркеры.
        let engine = Engine::parse(b"a\nb\nc");
        assert_eq!(engine.line_map.num_lines(), 3);
        assert_eq!(engine.line_map.newline_positions, vec![1, 3]);
    }
}
