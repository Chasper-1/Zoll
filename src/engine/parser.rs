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
//! Буфером владеет редактор; движок получает диапазоны в byte offsets.

use crate::engine::api::SpanSink;
use crate::engine::dependency::DependencyGraph;
use crate::engine::line_map::LineMap;
use crate::engine::resolver::{ResolveState, SyntaxSpan, process_marker};
use crate::engine::simd::scan;

// Набор интересующих байтов: синтаксические + структурный `\n`.
pub const INTERESTING_BYTES: &[u8] = b"*/_~=+-',$%!#>|:.)}@`\n";

// Движок парсера.
//
// Текст движок не хранит: парсинг — чистая функция «буфер → карта строк +
// спаны». Буфером владеет редактор; после каждой своей правки он отдаёт
// движку новый буфер через `reparse`. Спаны хранятся в одной копии —
// внутри графа зависимостей.
#[derive(Debug, Clone)]
pub struct Engine {
    // Номер версии документа (раздел 17 спеки).
    pub revision: u64,
    // Карта строк.
    pub line_map: LineMap,
    // Граф зависимостей спанов (единственная копия спанов).
    pub dependencies: DependencyGraph,
}

impl Engine {
    // Разобрать документ целиком.
    pub fn parse(text: &[u8]) -> Self {
        let (newline_positions, spans) = parse_document(text);
        Engine {
            revision: 0,
            line_map: LineMap::new(newline_positions),
            dependencies: DependencyGraph::new(spans),
        }
    }

    // Разобрать документ и сразу разослать спаны по ручке (fire-and-forget).
    // Спаны уходят по мере готовности: каждый — в момент создания.
    pub fn parse_into(text: &[u8], sink: &mut dyn SpanSink) -> Self {
        sink.begin_revision(0);
        let (newline_positions, spans) = parse_document_into(text, Some(sink));
        sink.end_revision();
        Engine {
            revision: 0,
            line_map: LineMap::new(newline_positions),
            dependencies: DependencyGraph::new(spans),
        }
    }

    // Пересобрать спаны из нового буфера редактора.
    //
    // Редактор сам применяет правку к своему буферу и отдаёт движку
    // результат. Инкремент revision на каждую пересборку. Сейчас
    // пересобирается весь документ; оптимизация «только затронутые
    // блоки» — следующий шаг.
    pub fn reparse(&mut self, text: &[u8]) -> &[SyntaxSpan] {
        self.revision += 1;
        let (newline_positions, spans) = parse_document(text);
        self.line_map = LineMap::new(newline_positions);
        self.dependencies = DependencyGraph::new(spans);
        self.dependencies.spans()
    }

    // Пересобрать и сразу разослать спаны по ручке (стрим, fire-and-forget).
    pub fn reparse_into(&mut self, text: &[u8], sink: &mut dyn SpanSink) -> &[SyntaxSpan] {
        self.revision += 1;
        sink.begin_revision(self.revision);
        let (newline_positions, spans) = parse_document_into(text, Some(sink));
        sink.end_revision();
        self.line_map = LineMap::new(newline_positions);
        self.dependencies = DependencyGraph::new(spans);
        self.dependencies.spans()
    }

    // Синтаксические диапазоны в порядке построения.
    pub fn spans(&self) -> &[SyntaxSpan] {
        self.dependencies.spans()
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
    parse_document_into(text, None)
}

// То же, но с синком: каждый спан отдаётся сразу в момент создания.
// begin_revision/end_revision — обязанность вызывающего (нужен номер версии).
pub(crate) fn parse_document_into(
    text: &[u8],
    sink: Option<&mut dyn SpanSink>,
) -> (Vec<usize>, Vec<SyntaxSpan>) {
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
    state.sink = sink;
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
                // Строка кончилась: line-close без `}` — спан до конца
                // строки (позиция `\n`), рождается здесь, уже финальный.
                if let Some((kind, open_position)) = state.pending_line_close.take() {
                    state.emit(SyntaxSpan {
                        start: open_position,
                        end: pos,
                        kind,
                    });
                }
                // Следующая строка — из готовой карты.
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
    // Последняя строка: line-close без `}` — спан до конца документа.
    if let Some((kind, open_position)) = state.pending_line_close.take() {
        state.emit(SyntaxSpan {
            start: open_position,
            end: state.line_end,
            kind,
        });
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
        assert_eq!(engine.spans().len(), 1);
        assert_eq!(engine.spans()[0].kind, SyntaxKind::Bold);
        assert_eq!(engine.revision, 0);
    }

    #[test]
    fn reparse_bumps_revision() {
        let mut engine = Engine::parse(b"hello");
        engine.reparse(b"hello world");
        assert_eq!(engine.revision, 1);
    }

    #[test]
    fn reparse_creates_new_spans() {
        let mut engine = Engine::parse(b"plain");
        assert!(engine.spans().is_empty());
        engine.reparse(b"**bold))");
        assert_eq!(engine.spans().len(), 1);
        assert_eq!(engine.spans()[0].kind, SyntaxKind::Bold);
    }

    #[test]
    fn line_at_after_reparse() {
        let mut engine = Engine::parse(b"a\nb");
        engine.reparse(b"a\nc\nb");
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
