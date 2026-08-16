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

use crate::engine::api::SpanSink;
use crate::engine::dependency::DependencyGraph;
use crate::engine::edit::Edit;
use crate::engine::line_map::LineMap;
use crate::engine::resolver::{ResolveState, SyntaxSpan, process_marker};
use crate::engine::simd::scan;

// Набор интересующих байтов: синтаксические + структурный `\n`.
pub const INTERESTING_BYTES: &[u8] = b"*/_~=+-',$%!#>|:.)}\n";

// Движок парсера.
#[derive(Debug, Clone)]
pub struct Engine {
    // Исходный буфер документа.
    pub text: Vec<u8>,
    // Номер версии документа (раздел 17 спеки).
    pub revision: u64,
    // Карта строк.
    pub line_map: LineMap,
    // Синтаксические диапазоны.
    pub spans: Vec<SyntaxSpan>,
    // Граф зависимостей спанов.
    pub dependencies: DependencyGraph,
}

impl Engine {
    // Разобрать документ целиком.
    pub fn parse(text: &[u8]) -> Self {
        let (newline_positions, spans) = parse_document(text);
        let dependencies = DependencyGraph::new(&spans);
        Engine {
            text: text.to_vec(),
            revision: 0,
            line_map: LineMap::new(newline_positions),
            spans,
            dependencies,
        }
    }

    // Разобрать документ и сразу разослать спаны по ручке (fire-and-forget).
    pub fn parse_into(text: &[u8], sink: &mut dyn SpanSink) -> Self {
        let engine = Self::parse(text);
        sink.on_spans(engine.revision, &engine.spans);
        engine
    }

    // Применить правку и пересобрать спаны.
    //
    // Инкремент revision на каждую правку. Сейчас пересобирается весь
    // документ; оптимизация «только затронутые блоки» — следующий шаг.
    pub fn apply_edit(&mut self, edit: &Edit) -> &[SyntaxSpan] {
        edit.apply(&mut self.text);
        self.revision += 1;

        let (newline_positions, spans) = parse_document(&self.text);
        self.line_map = LineMap::new(newline_positions);
        self.spans = spans;
        self.dependencies = DependencyGraph::new(&self.spans);
        &self.spans
    }

    // Применить правку и разослать новые спаны по ручке (fire-and-forget).
    pub fn apply_edit_into(&mut self, edit: &Edit, sink: &mut dyn SpanSink) -> &[SyntaxSpan] {
        // apply_edit инкрементит revision ровно на 1 — считаем заранее,
        // чтобы не читать self.revision под активным mutable-заимствованием.
        let revision = self.revision + 1;
        let spans = self.apply_edit(edit);
        sink.on_spans(revision, spans);
        spans
    }

    // Номер строки по байтовой позиции.
    pub fn line_at(&self, byte: usize) -> usize {
        self.line_map.line_at(byte)
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
    let mut line_idx = 0usize;

    // Текущий маркер: run подряд идущих одинаковых байтов.
    let mut run_byte: u8 = 0;
    let mut run_start: usize = 0;
    let mut run_len: usize = 0;

    scan(text, INTERESTING_BYTES, |offset, mask| {
        let mut remaining = mask;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let pos = offset + bit;
            let byte = text[pos];
            remaining &= remaining - 1;

            // Продолжение текущего маркера.
            if byte == run_byte && pos == run_start + run_len {
                run_len += 1;
                continue;
            }
            // Завершаем предыдущий маркер и разбираем его.
            if run_len > 0 {
                process_marker(&mut state, run_byte, run_start, run_len);
                run_len = 0;
            }
            if byte == b'\n' {
                // Строка кончилась — берём следующую из готовой карты.
                line_idx += 1;
                state.line_start = pos + 1;
                state.line_end = newline_positions
                    .get(line_idx)
                    .copied()
                    .unwrap_or(text.len());
                // Inline и line-level не выходят за строку — сбрасываем.
                state.inline_stack.clear();
                state.line_stack.clear();
            } else {
                run_byte = byte;
                run_start = pos;
                run_len = 1;
            }
        }
    });
    // Последний маркер документа.
    if run_len > 0 {
        process_marker(&mut state, run_byte, run_start, run_len);
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
        engine.apply_edit(&Edit::new(5, 0, b" world"));
        assert_eq!(engine.revision, 1);
        assert_eq!(engine.text, b"hello world");
    }

    #[test]
    fn edit_creates_new_spans() {
        let mut engine = Engine::parse(b"plain");
        assert!(engine.spans.is_empty());
        engine.apply_edit(&Edit::new(0, 0, b"**bold))"));
        assert_eq!(engine.spans.len(), 1);
        assert_eq!(engine.spans[0].kind, SyntaxKind::Bold);
    }

    #[test]
    fn line_at_after_edit() {
        let mut engine = Engine::parse(b"a\nb");
        engine.apply_edit(&Edit::new(2, 0, b"c\n"));
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
