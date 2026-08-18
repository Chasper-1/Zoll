// API-слой (ручки): контракт движок <-> редактор.
//
// Единый язык — байтовые координаты: SyntaxSpan и правки говорят
// в absolute byte offsets, без юникод-символов, строк и деревьев.
//
// Отправка результатов — «выстрелил и забыл» (спека, этап 3):
// движок вызывает ручку и не ждёт ответа. Клиент (редактор) принимает
// обновления и отправляет правки обратно через EngineHandle.
//
// Ручки — по одной на каждую конструкцию: редактор получает уже
// типизированные вызовы (on_bold, on_header с уровнем и т.д.) и не
// определяет тип сам. Пачка вызовов между begin_revision/end_revision —
// одна версия документа.

use super::Engine;
use super::resolver::SyntaxKind;

pub use super::SyntaxSpan;

// Ручка на получателя спанов. Реализуется редактором.
//
// Движок не ждёт ответа от этой ручки: реализация должна принять
// пачку вызовов и вернуться как можно быстрее.
//
// Пачка: begin_revision(revision) → по одному вызову на каждую
// конструкцию → end_revision(). Все вызовы между begin/end — одна
// версия документа.
//
// Все методы обязательны: никаких «тихих» пропусков — если редактор
// не реализовал ручку, это ошибка компиляции, а не молча потерянные
// данные.
pub trait SpanSink {
    // Начало пачки: все последующие вызовы — из одной версии.
    fn begin_revision(&mut self, revision: u64);
    // ─── Inline ───
    fn on_bold(&mut self, start: usize, end: usize);
    fn on_italic(&mut self, start: usize, end: usize);
    fn on_underline(&mut self, start: usize, end: usize);
    fn on_strikethrough(&mut self, start: usize, end: usize);
    fn on_highlight(&mut self, start: usize, end: usize);
    fn on_insertion(&mut self, start: usize, end: usize);
    fn on_deletion(&mut self, start: usize, end: usize);
    fn on_superscript(&mut self, start: usize, end: usize);
    fn on_subscript(&mut self, start: usize, end: usize);
    fn on_formula_inline(&mut self, start: usize, end: usize);
    fn on_comment_inline(&mut self, start: usize, end: usize);
    fn on_spoiler_inline(&mut self, start: usize, end: usize);
    fn on_code_inline(&mut self, start: usize, end: usize);
    // ─── Line ───
    fn on_header(&mut self, start: usize, end: usize, level: u32);
    fn on_tag(&mut self, start: usize, end: usize);
    fn on_quote(&mut self, start: usize, end: usize);
    fn on_list_item(&mut self, start: usize, end: usize);
    fn on_table_row(&mut self, start: usize, end: usize);
    fn on_thematic_break(&mut self, start: usize, end: usize);
    fn on_formula_line(&mut self, start: usize, end: usize);
    fn on_comment_line(&mut self, start: usize, end: usize);
    fn on_spoiler_line(&mut self, start: usize, end: usize);
    fn on_code_line(&mut self, start: usize, end: usize);
    // ─── Block ───
    fn on_formula_block(&mut self, start: usize, end: usize);
    fn on_comment_block(&mut self, start: usize, end: usize);
    fn on_spoiler_block(&mut self, start: usize, end: usize);
    fn on_code_block(&mut self, start: usize, end: usize);
    fn on_metadata(&mut self, start: usize, end: usize);
    // Конец пачки.
    fn end_revision(&mut self);
}

// Отдаёт один спан по ручке: match по своему enum (джамп-таблица),
// редактор получает уже типизированный вызов. Общий код для стрима
// (спаны уходят по мере создания) и батча (все разом после парсинга).
#[inline]
pub(crate) fn dispatch_span(sink: &mut dyn SpanSink, span: SyntaxSpan) {
    match span.kind {
        SyntaxKind::Bold => sink.on_bold(span.start, span.end),
        SyntaxKind::Italic => sink.on_italic(span.start, span.end),
        SyntaxKind::Underline => sink.on_underline(span.start, span.end),
        SyntaxKind::Strikethrough => sink.on_strikethrough(span.start, span.end),
        SyntaxKind::Highlight => sink.on_highlight(span.start, span.end),
        SyntaxKind::Insertion => sink.on_insertion(span.start, span.end),
        SyntaxKind::Deletion => sink.on_deletion(span.start, span.end),
        SyntaxKind::Superscript => sink.on_superscript(span.start, span.end),
        SyntaxKind::Subscript => sink.on_subscript(span.start, span.end),
        SyntaxKind::FormulaInline => sink.on_formula_inline(span.start, span.end),
        SyntaxKind::CommentInline => sink.on_comment_inline(span.start, span.end),
        SyntaxKind::SpoilerInline => sink.on_spoiler_inline(span.start, span.end),
        SyntaxKind::CodeInline => sink.on_code_inline(span.start, span.end),
        SyntaxKind::Header(level) => sink.on_header(span.start, span.end, level),
        SyntaxKind::Tag => sink.on_tag(span.start, span.end),
        SyntaxKind::Quote => sink.on_quote(span.start, span.end),
        SyntaxKind::ListItem => sink.on_list_item(span.start, span.end),
        SyntaxKind::TableRow => sink.on_table_row(span.start, span.end),
        SyntaxKind::ThematicBreak => sink.on_thematic_break(span.start, span.end),
        SyntaxKind::FormulaLine => sink.on_formula_line(span.start, span.end),
        SyntaxKind::CommentLine => sink.on_comment_line(span.start, span.end),
        SyntaxKind::SpoilerLine => sink.on_spoiler_line(span.start, span.end),
        SyntaxKind::CodeLine => sink.on_code_line(span.start, span.end),
        SyntaxKind::FormulaBlock => sink.on_formula_block(span.start, span.end),
        SyntaxKind::CommentBlock => sink.on_comment_block(span.start, span.end),
        SyntaxKind::SpoilerBlock => sink.on_spoiler_block(span.start, span.end),
        SyntaxKind::CodeBlock => sink.on_code_block(span.start, span.end),
        SyntaxKind::Metadata => sink.on_metadata(span.start, span.end),
    }
}

// Рассылает спаны по ручке: одна пачка на версию документа.
//
// Батч-режим: спаны уже готовы (движок разобрал документ целиком),
// отдаются все разом. Стрим-режим — `parse_into`/`reparse_into`:
// те же вызовы, но по мере готовности.
pub fn dispatch_spans(sink: &mut dyn SpanSink, revision: u64, spans: &[SyntaxSpan]) {
    sink.begin_revision(revision);
    for span in spans {
        dispatch_span(sink, *span);
    }
    sink.end_revision();
}

// Ручка на движок — единственный объект, который держит редактор.
pub struct EngineHandle {
    engine: Engine,
}

impl EngineHandle {
    // Разобрать документ с нуля (revision = 0).
    pub fn parse(text: &[u8]) -> Self {
        EngineHandle {
            engine: Engine::parse(text),
        }
    }

    // Текущая версия документа.
    pub fn revision(&self) -> u64 {
        self.engine.revision
    }

    // Текущие синтаксические диапазоны (в байтах).
    pub fn spans(&self) -> &[SyntaxSpan] {
        self.engine.spans()
    }

    // Номер строки (0-based) по байтовой позиции.
    pub fn line_at(&self, byte: usize) -> usize {
        self.engine.line_at(byte)
    }

    // Спаны, полностью лежащие в [start, end).
    pub fn spans_in(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.engine.dependencies.spans_in(start, end)
    }

    // Спаны, пересекающие [start, end).
    pub fn spans_overlapping(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.engine.dependencies.spans_overlapping(start, end)
    }

    // Пересобрать спаны из нового буфера редактора и разослать по ручке.
    //
    // Редактор сам применил правку к своему буферу и отдаёт результат.
    // Fire-and-forget: результат уходит в sink, движок не ждёт ответа.
    pub fn reparse(&mut self, text: &[u8], sink: &mut dyn SpanSink) -> &[SyntaxSpan] {
        self.engine.reparse_into(text, sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SyntaxKind;

    // Записывает все вызовы ручки для проверки: собирает спаны обратно
    // из типизированных вызовов.
    struct RecordingSink {
        batches: Vec<(u64, Vec<SyntaxSpan>)>,
    }

    impl SpanSink for RecordingSink {
        fn begin_revision(&mut self, revision: u64) {
            self.batches.push((revision, Vec::new()));
        }
        fn on_bold(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Bold,
            });
        }
        fn on_italic(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Italic,
            });
        }
        fn on_underline(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Underline,
            });
        }
        fn on_strikethrough(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Strikethrough,
            });
        }
        fn on_highlight(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Highlight,
            });
        }
        fn on_insertion(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Insertion,
            });
        }
        fn on_deletion(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Deletion,
            });
        }
        fn on_superscript(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Superscript,
            });
        }
        fn on_subscript(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Subscript,
            });
        }
        fn on_formula_inline(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::FormulaInline,
            });
        }
        fn on_comment_inline(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CommentInline,
            });
        }
        fn on_spoiler_inline(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::SpoilerInline,
            });
        }
        fn on_code_inline(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CodeInline,
            });
        }
        fn on_header(&mut self, start: usize, end: usize, level: u32) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Header(level),
            });
        }
        fn on_tag(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Tag,
            });
        }
        fn on_quote(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Quote,
            });
        }
        fn on_list_item(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::ListItem,
            });
        }
        fn on_table_row(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::TableRow,
            });
        }
        fn on_thematic_break(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::ThematicBreak,
            });
        }
        fn on_formula_line(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::FormulaLine,
            });
        }
        fn on_comment_line(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CommentLine,
            });
        }
        fn on_spoiler_line(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::SpoilerLine,
            });
        }
        fn on_code_line(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CodeLine,
            });
        }
        fn on_formula_block(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::FormulaBlock,
            });
        }
        fn on_comment_block(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CommentBlock,
            });
        }
        fn on_spoiler_block(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::SpoilerBlock,
            });
        }
        fn on_code_block(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::CodeBlock,
            });
        }
        fn on_metadata(&mut self, start: usize, end: usize) {
            self.batches.last_mut().unwrap().1.push(SyntaxSpan {
                start,
                end,
                kind: SyntaxKind::Metadata,
            });
        }
        fn end_revision(&mut self) {}
    }

    #[test]
    fn parse_into_dispatches_once() {
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        let engine = Engine::parse_into("**жирный))".as_bytes(), &mut sink);
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(sink.batches[0].0, 0);
        assert_eq!(sink.batches[0].1.len(), 1);
        assert_eq!(sink.batches[0].1[0].kind, SyntaxKind::Bold);
        assert_eq!(engine.spans().len(), 1);
    }

    #[test]
    fn reparse_dispatches_with_bumped_revision() {
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        let mut handle = EngineHandle::parse(b"plain");
        handle.reparse(b"**bold))", &mut sink);
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(sink.batches[0].0, 1);
        assert_eq!(sink.batches[0].1[0].kind, SyntaxKind::Bold);
    }

    #[test]
    fn reparse_empty_dispatches_with_bumped_revision() {
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        let mut handle = EngineHandle::parse(b"**bold))");
        handle.reparse(b"", &mut sink);
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(sink.batches[0].0, 1);
        assert!(sink.batches[0].1.is_empty());
    }

    #[test]
    fn engine_handle_roundtrip() {
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        let mut handle = EngineHandle::parse(b"**a))");
        handle.reparse(b"**a)) b", &mut sink);
        assert_eq!(handle.revision(), 1);
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(sink.batches[0].0, 1);
        assert_eq!(handle.spans_in(0, 8).len(), 1);
    }

    #[test]
    fn typed_handles_receive_own_kind() {
        // Каждая конструкция приходит в свою ручку: заголовок — с уровнем,
        // комментарии трёх уровней — в три разные ручки.
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        Engine::parse_into(
            "#1 Заголовок\n%скрыто))\n%%скрыто}\n%%%\nблок\n}\n`код))\n``строка}\n```\nблок\n}\n"
                .as_bytes(),
            &mut sink,
        );
        let kinds: Vec<SyntaxKind> = sink.batches[0].1.iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SyntaxKind::Header(1),
                SyntaxKind::CommentInline,
                SyntaxKind::CommentLine,
                SyntaxKind::CommentBlock,
                SyntaxKind::CodeInline,
                SyntaxKind::CodeLine,
                SyntaxKind::CodeBlock,
            ]
        );
    }

    // ─── Стрим: спаны уходят по мере готовности ─────────────────

    #[test]
    fn stream_block_arrives_after_inner_spans() {
        // Блок отдаётся в момент `}` в начале строки — после спанов строк
        // внутри него (line-close закрывается своей `}` раньше).
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        Engine::parse_into("%%%\n%%скрыто}\n}".as_bytes(), &mut sink);
        let kinds: Vec<SyntaxKind> = sink.batches[0].1.iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![SyntaxKind::CommentLine, SyntaxKind::CommentBlock]
        );
    }

    #[test]
    fn stream_line_close_flushed_at_eol() {
        // Line-close без `}` отдаётся в конце строки, спан до позиции \n.
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        Engine::parse_into("%%скрыто\n".as_bytes(), &mut sink);
        assert_eq!(sink.batches[0].1.len(), 1);
        assert_eq!(sink.batches[0].1[0].kind, SyntaxKind::CommentLine);
        assert_eq!(
            (sink.batches[0].1[0].start, sink.batches[0].1[0].end),
            (0, 14)
        );
    }

    #[test]
    fn stream_empty_document() {
        // Пустой документ: одна пустая пачка (begin/end), без спанов.
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        Engine::parse_into(b"", &mut sink);
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(sink.batches[0].0, 0);
        assert!(sink.batches[0].1.is_empty());
    }

    #[test]
    fn stream_matches_batch_order() {
        // Стрим и батч отдают спаны в одном порядке (порядок создания).
        let text = "#1 Заголовок\n**жирный)) %%скрыто}\n%%%\nблок\n}\n";
        let mut sink = RecordingSink {
            batches: Vec::new(),
        };
        Engine::parse_into(text.as_bytes(), &mut sink);
        let streamed: Vec<SyntaxSpan> = sink.batches[0].1.clone();
        let batched = Engine::parse(text.as_bytes()).spans().to_vec();
        assert_eq!(streamed, batched);
    }
}
