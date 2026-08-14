// API-слой (ручки): контракт движок <-> редактор.
//
// Единый язык — байтовые координаты: SyntaxSpan и Edit говорят
// в absolute byte offsets, без юникод-символов, строк и деревьев.
//
// Отправка результатов — «выстрелил и забыл» (спека, этап 3):
// движок вызывает SpanSink::on_spans и не ждёт ответа.
// Клиент (редактор) принимает обновления и отправляет правки
// обратно через EngineHandle.

use super::Engine;

pub use super::{Edit, SyntaxSpan};

// Ручка на получателя спанов. Реализуется редактором.
//
// Движок не ждёт ответа от этой ручки: реализация должна принять
// (revision, spans) и вернуться как можно быстрее.
pub trait SpanSink {
    // Движок пересобрал документ.
    fn on_spans(&mut self, revision: u64, spans: &[SyntaxSpan]);
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
        &self.engine.spans
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

    // Применить правку, пересобрать спаны и разослать их по ручкам.
    //
    // Fire-and-forget: результат уходит в sink, движок не ждёт ответа.
    pub fn apply_edit(&mut self, edit: &Edit, sink: &mut dyn SpanSink) {
        self.engine.apply_edit_into(edit, sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SyntaxKind;

    // Записывает все вызовы ручки для проверки.
    struct RecordingSink {
        calls: Vec<(u64, Vec<SyntaxSpan>)>,
    }

    impl SpanSink for RecordingSink {
        fn on_spans(&mut self, revision: u64, spans: &[SyntaxSpan]) {
            self.calls.push((revision, spans.to_vec()));
        }
    }

    #[test]
    fn parse_into_dispatches_once() {
        let mut sink = RecordingSink { calls: Vec::new() };
        let engine = Engine::parse_into("**жирный))".as_bytes(), &mut sink);
        assert_eq!(sink.calls.len(), 1);
        assert_eq!(sink.calls[0].0, 0);
        assert_eq!(sink.calls[0].1.len(), 1);
        assert_eq!(sink.calls[0].1[0].kind, SyntaxKind::Bold);
        assert_eq!(engine.spans.len(), 1);
    }

    #[test]
    fn apply_edit_dispatches_with_bumped_revision() {
        let mut sink = RecordingSink { calls: Vec::new() };
        let mut engine = Engine::parse_into(b"plain", &mut sink);
        sink.calls.clear();
        engine.apply_edit_into(&Edit::new(0, 0, b"**bold))"), &mut sink);
        assert_eq!(sink.calls.len(), 1);
        assert_eq!(sink.calls[0].0, 1);
        assert_eq!(sink.calls[0].1[0].kind, SyntaxKind::Bold);
    }

    #[test]
    fn engine_handle_roundtrip() {
        let mut sink = RecordingSink { calls: Vec::new() };
        let mut handle = EngineHandle::parse(b"**a))");
        handle.apply_edit(&Edit::new(5, 0, b" b"), &mut sink);
        assert_eq!(handle.revision(), 1);
        assert_eq!(sink.calls.len(), 1);
        assert_eq!(sink.calls[0].0, 1);
        assert_eq!(handle.spans_in(0, 8).len(), 1);
    }
}
