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

// Набор интересующих байтов: только синтаксические маркеры. `\n` не нужен:
// конец строки определяется по карте строк (этап 1) — одно сравнение
// вместо события в цикле. `:` и `.` не маркеры — в скане не участвуют.
pub const INTERESTING_BYTES: &[u8] = b"*/_~=+-',$%!#>|;)}@`";

// Накопитель маркерного run'а: объединяет подряд идущие одинаковые байты
// в один маркер. SIMD-скан отдаёт одиночные байты; склейка — задача
// этого слоя. Извлекает байт и позицию из mask-события scan().
struct MarkerAccumulator {
    byte: u8,
    start: usize,
    end: usize,
    len: usize,
}

impl MarkerAccumulator {
    fn new() -> Self {
        MarkerAccumulator {
            byte: 0,
            start: 0,
            end: 0,
            len: 0,
        }
    }

    // Пробует добавить байт в текущий run.
    // Возвращает `true` если run продолжается (байт == предыдущий и позиция
    // смежная), `false` если run оборван — вызывающий должен завершить
    // предыдущий маркер и начать новый через `start_new`.
    #[inline(always)]
    fn push(&mut self, byte: u8, pos: usize) -> bool {
        if pos == self.end && byte == self.byte {
            self.len += 1;
            self.end += 1;
            true
        } else {
            false
        }
    }

    // Начинает новый маркерный run.
    #[inline(always)]
    fn start_new(&mut self, byte: u8, pos: usize) {
        self.byte = byte;
        self.start = pos;
        self.end = pos + 1;
        self.len = 1;
    }
}

// Категории байтов для проверок в горячем цикле: вместо `is_ascii_digit()`
// и `if byte == ...` — один индекс в таблицу (256 байт, L1).
pub(crate) const CAT_DIGIT: u8 = 1;
pub(crate) const CAT_OTHER: u8 = 0;

// Таблица категорий: 1 — цифра, 0 — всё остальное.
pub(crate) const CATEGORY: [u8; 256] = {
    let mut table = [CAT_OTHER; 256];
    let mut i = b'0' as usize;
    while i <= b'9' as usize {
        table[i] = CAT_DIGIT;
        i += 1;
    }
    table
};

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
    // Generic: компилятор мономорфизирует dispatch_span для конкретного
    // типа sink, убирая vtable-диспетчеризацию из горячего пути.
    pub fn parse_into<S: SpanSink + ?Sized>(text: &[u8], sink: &mut S) -> Self {
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
    pub fn reparse_into<S: SpanSink + ?Sized>(
        &mut self,
        text: &[u8],
        sink: &mut S,
    ) -> &[SyntaxSpan] {
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
// Маркерный тип для batch-режима: sink не используется, нужен только
// для вывода типа S в parse_document_into. Компилятор удаляет весь
// код NoSink, т.к. sink = None и методы не вызываются.
struct NoSink;

impl SpanSink for NoSink {
    fn begin_revision(&mut self, _: u64) {}
    fn on_bold(&mut self, _: usize, _: usize) {}
    fn on_italic(&mut self, _: usize, _: usize) {}
    fn on_underline(&mut self, _: usize, _: usize) {}
    fn on_strikethrough(&mut self, _: usize, _: usize) {}
    fn on_highlight(&mut self, _: usize, _: usize) {}
    fn on_insertion(&mut self, _: usize, _: usize) {}
    fn on_deletion(&mut self, _: usize, _: usize) {}
    fn on_superscript(&mut self, _: usize, _: usize) {}
    fn on_subscript(&mut self, _: usize, _: usize) {}
    fn on_formula_inline(&mut self, _: usize, _: usize) {}
    fn on_comment_inline(&mut self, _: usize, _: usize) {}
    fn on_spoiler_inline(&mut self, _: usize, _: usize) {}
    fn on_code_inline(&mut self, _: usize, _: usize) {}
    fn on_header(&mut self, _: usize, _: usize, _: u8) {}
    fn on_tag(&mut self, _: usize, _: usize) {}
    fn on_quote(&mut self, _: usize, _: usize) {}
    fn on_list_item(&mut self, _: usize, _: usize) {}
    fn on_table_row(&mut self, _: usize, _: usize) {}
    fn on_thematic_break(&mut self, _: usize, _: usize) {}
    fn on_formula_line(&mut self, _: usize, _: usize) {}
    fn on_comment_line(&mut self, _: usize, _: usize) {}
    fn on_spoiler_line(&mut self, _: usize, _: usize) {}
    fn on_code_line(&mut self, _: usize, _: usize) {}
    fn on_formula_block(&mut self, _: usize, _: usize) {}
    fn on_comment_block(&mut self, _: usize, _: usize) {}
    fn on_spoiler_block(&mut self, _: usize, _: usize) {}
    fn on_code_block(&mut self, _: usize, _: usize) {}
    fn on_metadata(&mut self, _: usize, _: usize) {}
    fn end_revision(&mut self) {}
}

pub(crate) fn parse_document(text: &[u8]) -> (Vec<usize>, Vec<SyntaxSpan>) {
    parse_document_into::<NoSink>(text, None)
}
// То же, но с синком: каждый спан отдаётся сразу в момент создания.
// begin_revision/end_revision — обязанность вызывающего (нужен номер версии).
// Generic: компилятор мономорфизирует для конкретного типа sink (или
// использует din-путь для dyn SpanSink), убирая виртуальные вызовы.
pub(crate) fn parse_document_into<S: SpanSink + ?Sized>(
    text: &[u8],
    sink: Option<&mut S>,
) -> (Vec<usize>, Vec<SyntaxSpan>) {
    // ─── Этап 1: регистры SIMD → готовые строки ───
    // Сразу с запасом: 1/16 документа (средняя строка ~30-80 байт) —
    // без перевыделений на любом разумном документе.
    let mut newline_positions: Vec<usize> = Vec::with_capacity(text.len() / 16 + 16);
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
    // Локальная копия границы строки: проверка в регистре, без загрузки
    // поля и bounds check на каждое событие.
    let mut line_end = state.line_end;

    let mut marker = MarkerAccumulator::new();

    scan(text, INTERESTING_BYTES, |offset, mask| {
        let mut remaining = mask;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let pos = offset + bit;
            let byte = text[pos];
            remaining &= remaining - 1;

            // Продолжение текущего маркерного run'а или завершение
            // предыдущего + начало нового.
            if !marker.push(byte, pos) {
                // Завершаем предыдущий маркер и разбираем его.
                if marker.len > 0 {
                    process_marker(&mut state, marker.byte, marker.start, marker.len);
                }
                marker.start_new(byte, pos);
            }

            // Строка кончилась (позиция за картой `\n`): line-close без
            // `}` — спан до конца строки, рождается здесь, уже финальный.
            // Проверка по карте строк вместо `\n`-события в скане: `\n`
            // убран из INTERESTING_BYTES, конец строки — одно сравнение.
            // После завершения маркера: inline старой строки успевает
            // попасть в стек до его сброса.
            if pos > line_end {
                while line_index < newline_positions.len() && pos > newline_positions[line_index] {
                    if let Some((kind, open_position)) = state.pending_line_close.take() {
                        state.emit(SyntaxSpan {
                            start: open_position,
                            end: newline_positions[line_index],
                            kind,
                        });
                    }
                    // Следующая строка — из готовой карты.
                    line_index += 1;
                    state.line_start = newline_positions[line_index - 1] + 1;
                    line_end = newline_positions
                        .get(line_index)
                        .copied()
                        .unwrap_or(text.len());
                    state.line_end = line_end;
                    // Inline не выходит за строку — сбрасываем.
                    state.inline_stack.clear();
                }
            }
        }
    });
    // Последний маркер документа.
    if marker.len > 0 {
        process_marker(&mut state, marker.byte, marker.start, marker.len);
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
    fn marker_across_block_boundary() {
        // Маркер, пересекающий границу 32-байтного блока скана:
        // `**` на позициях 31-32, `))` на 37-38.
        let text = format!("{}**bold))", "a".repeat(31));
        let engine = Engine::parse(text.as_bytes());
        assert_eq!(engine.spans().len(), 1);
        assert_eq!(engine.spans()[0].kind, SyntaxKind::Bold);
        assert_eq!((engine.spans()[0].start, engine.spans()[0].end), (31, 39));
    }

    #[test]
    fn triple_marker_across_block_boundary() {
        // `%%%` на позициях 31-33 (в начале строки): run через границу
        // блока, закрытие `}` в начале следующей строки.
        let text = format!("{}\n%%%блок\n}}", "a".repeat(30));
        let engine = Engine::parse(text.as_bytes());
        assert_eq!(engine.spans().len(), 1);
        assert_eq!(engine.spans()[0].kind, SyntaxKind::CommentBlock);
        assert_eq!((engine.spans()[0].start, engine.spans()[0].end), (31, 44));
    }

    #[test]
    fn line_map_built_in_one_pass() {
        // Карта строк строится в том же проходе, что и маркеры.
        let engine = Engine::parse(b"a\nb\nc");
        assert_eq!(engine.line_map.num_lines(), 3);
        assert_eq!(engine.line_map.newline_positions, vec![1, 3]);
    }

    // ─── Edge cases ─────────────────────────────────────────────

    #[test]
    fn many_markers_on_one_line() {
        // Длинная строка с >100 inline-маркерами.
        let mut text = String::new();
        for _ in 0..150 {
            text.push_str("**жирный)) ");
        }
        let engine = Engine::parse(text.as_bytes());
        assert_eq!(engine.spans().len(), 150);
        assert!(engine.spans().iter().all(|s| s.kind == SyntaxKind::Bold));
    }

    #[test]
    fn document_entirely_markers() {
        // Документ без обычного текста: только маркеры и закрытия.
        let text = "**bold)) //italic))\n$$formula}\n%%comment}\n---\n";
        let engine = Engine::parse(text.as_bytes());
        let kinds: Vec<SyntaxKind> = engine.spans().iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Bold));
        assert!(kinds.contains(&SyntaxKind::Italic));
        assert!(kinds.contains(&SyntaxKind::FormulaLine));
        assert!(kinds.contains(&SyntaxKind::CommentLine));
        assert!(kinds.contains(&SyntaxKind::ThematicBreak));
    }

    #[test]
    fn empty_document_parse_into() {
        // Пустой документ через стрим — без спанов.
        use crate::engine::api::SpanSink;
        struct Empty;
        impl SpanSink for Empty {
            fn begin_revision(&mut self, _: u64) {}
            fn on_bold(&mut self, _: usize, _: usize) {}
            fn on_italic(&mut self, _: usize, _: usize) {}
            fn on_underline(&mut self, _: usize, _: usize) {}
            fn on_strikethrough(&mut self, _: usize, _: usize) {}
            fn on_highlight(&mut self, _: usize, _: usize) {}
            fn on_insertion(&mut self, _: usize, _: usize) {}
            fn on_deletion(&mut self, _: usize, _: usize) {}
            fn on_superscript(&mut self, _: usize, _: usize) {}
            fn on_subscript(&mut self, _: usize, _: usize) {}
            fn on_formula_inline(&mut self, _: usize, _: usize) {}
            fn on_comment_inline(&mut self, _: usize, _: usize) {}
            fn on_spoiler_inline(&mut self, _: usize, _: usize) {}
            fn on_code_inline(&mut self, _: usize, _: usize) {}
            fn on_header(&mut self, _: usize, _: usize, _: u8) {}
            fn on_tag(&mut self, _: usize, _: usize) {}
            fn on_quote(&mut self, _: usize, _: usize) {}
            fn on_list_item(&mut self, _: usize, _: usize) {}
            fn on_table_row(&mut self, _: usize, _: usize) {}
            fn on_thematic_break(&mut self, _: usize, _: usize) {}
            fn on_formula_line(&mut self, _: usize, _: usize) {}
            fn on_comment_line(&mut self, _: usize, _: usize) {}
            fn on_spoiler_line(&mut self, _: usize, _: usize) {}
            fn on_code_line(&mut self, _: usize, _: usize) {}
            fn on_formula_block(&mut self, _: usize, _: usize) {}
            fn on_comment_block(&mut self, _: usize, _: usize) {}
            fn on_spoiler_block(&mut self, _: usize, _: usize) {}
            fn on_code_block(&mut self, _: usize, _: usize) {}
            fn on_metadata(&mut self, _: usize, _: usize) {}
            fn end_revision(&mut self) {}
        }
        let mut sink = Empty;
        let engine = Engine::parse_into(b"", &mut sink);
        assert_eq!(engine.spans().len(), 0);
    }

    #[test]
    fn stream_span_order_at_block_boundaries() {
        // Стрим: блочный %%% закрывается после line-level %% внутри него.
        use crate::engine::api::SpanSink;
        struct Collector {
            kinds: Vec<SyntaxKind>,
        }
        impl Collector {
            fn new() -> Self {
                Collector { kinds: Vec::new() }
            }
        }
        impl SpanSink for Collector {
            fn begin_revision(&mut self, _: u64) {}
            fn on_bold(&mut self, _: usize, _: usize) {}
            fn on_italic(&mut self, _: usize, _: usize) {}
            fn on_underline(&mut self, _: usize, _: usize) {}
            fn on_strikethrough(&mut self, _: usize, _: usize) {}
            fn on_highlight(&mut self, _: usize, _: usize) {}
            fn on_insertion(&mut self, _: usize, _: usize) {}
            fn on_deletion(&mut self, _: usize, _: usize) {}
            fn on_superscript(&mut self, _: usize, _: usize) {}
            fn on_subscript(&mut self, _: usize, _: usize) {}
            fn on_formula_inline(&mut self, _: usize, _: usize) {}
            fn on_comment_inline(&mut self, _: usize, _: usize) {}
            fn on_spoiler_inline(&mut self, _: usize, _: usize) {}
            fn on_code_inline(&mut self, _: usize, _: usize) {}
            fn on_header(&mut self, _: usize, _: usize, _: u8) {}
            fn on_tag(&mut self, _: usize, _: usize) {}
            fn on_quote(&mut self, _: usize, _: usize) {}
            fn on_list_item(&mut self, _: usize, _: usize) {}
            fn on_table_row(&mut self, _: usize, _: usize) {}
            fn on_thematic_break(&mut self, _: usize, _: usize) {}
            fn on_formula_line(&mut self, _: usize, _: usize) {}
            fn on_comment_line(&mut self, _: usize, _: usize) {
                self.kinds.push(SyntaxKind::CommentLine);
            }
            fn on_spoiler_line(&mut self, _: usize, _: usize) {}
            fn on_code_line(&mut self, _: usize, _: usize) {}
            fn on_formula_block(&mut self, _: usize, _: usize) {}
            fn on_comment_block(&mut self, _: usize, _: usize) {
                self.kinds.push(SyntaxKind::CommentBlock);
            }
            fn on_spoiler_block(&mut self, _: usize, _: usize) {}
            fn on_code_block(&mut self, _: usize, _: usize) {}
            fn on_metadata(&mut self, _: usize, _: usize) {}
            fn end_revision(&mut self) {}
        }
        let mut sink = Collector::new();
        Engine::parse_into("%%%\n%%скрыто}\n}".as_bytes(), &mut sink);
        // CommentLine приходит раньше CommentBlock (line-close внутри блока
        // закрывается своей `}` раньше, чем блок закроется своей `}`).
        assert_eq!(
            sink.kinds,
            vec![SyntaxKind::CommentLine, SyntaxKind::CommentBlock]
        );
    }
}
