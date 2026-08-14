//! Движок-парсер Zoll по спецификации `docs/parser.md`.
//!
//! Пайплайн:
//!
//! ```text
//! байты документа
//!     ↓
//! SIMD-поиск значимых байтов      (simd::scan)
//!     ↓
//! позиции найденных байтов + \n
//!     ↓
//! карта строк                     (line_map::LineMap)
//!     ↓
//! сборка маркеров                 (collector::collect)
//!     ↓
//! проверка синтаксических правил  (resolver::resolve)
//!     ↓
//! синтаксические диапазоны        (SyntaxSpan)
//! ```
//!
//! Единая координатная система — абсолютная позиция в байтах.
//! Редактор получает исходный буфер и диапазоны в byte offsets.

mod collector;
mod dependency;
mod edit;
mod line_map;
mod resolver;
mod simd;

pub use collector::{Marker, collect};
pub use dependency::DependencyGraph;
pub use edit::Edit;
pub use line_map::LineMap;
pub use resolver::{SyntaxKind, SyntaxSpan, resolve};
pub use simd::{Event, scan};

/// Набор интересующих байтов: синтаксические + структурный `\n`.
pub const INTERESTING_BYTES: &[u8] = b"*/_~=+-',$%!#>|:)}\n";

/// Движок парсера.
#[derive(Debug, Clone)]
pub struct Engine {
    /// Исходный буфер документа.
    pub text: Vec<u8>,
    /// Номер версии документа (раздел 17 спеки).
    pub revision: u64,
    /// Карта строк.
    pub line_map: LineMap,
    /// Синтаксические диапазоны.
    pub spans: Vec<SyntaxSpan>,
    /// Граф зависимостей спанов.
    pub dependencies: DependencyGraph,
}

impl Engine {
    /// Разобрать документ целиком.
    pub fn parse(text: &[u8]) -> Self {
        let events = scan(text, INTERESTING_BYTES);
        let line_map = LineMap::from_events(&events);
        let markers = collect(&events);
        let spans = resolve(text, &markers, &line_map);
        let dependencies = DependencyGraph::new(&spans);
        Engine {
            text: text.to_vec(),
            revision: 0,
            line_map,
            spans,
            dependencies,
        }
    }

    /// Применить правку и пересобрать спаны.
    ///
    /// Инкремент revision на каждую правку. Сейчас пересобирается весь
    /// документ; оптимизация «только затронутые блоки» — следующий шаг.
    pub fn apply_edit(&mut self, edit: &Edit) -> &[SyntaxSpan] {
        edit.apply(&mut self.text);
        self.revision += 1;

        let events = scan(&self.text, INTERESTING_BYTES);
        self.line_map = LineMap::from_events(&events);
        let markers = collect(&events);
        self.spans = resolve(&self.text, &markers, &self.line_map);
        self.dependencies = DependencyGraph::new(&self.spans);
        &self.spans
    }

    /// Номер строки по байтовой позиции.
    pub fn line_at(&self, byte: usize) -> usize {
        self.line_map.line_at(byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
