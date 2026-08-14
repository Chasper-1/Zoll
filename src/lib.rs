//! Zoll markup language — чистый парсер разметки.
//!
//! Зависимостей нет, только `std`.
//!
//! - [`ast`] — AST (MarkupDoc, MarkupNode, MarkupStyle, LineAST)
//! - [`parser`] — строчный парсер (parse_line, merge)
//! - [`incremental`] — инкрементальный парсер (IncrementalDoc)
//! - [`viewport`] — ленивый парсинг по видимому диапазону
//! - [`engine`] — движок по спецификации `docs/parser.md` (SIMD-скан → спаны)

pub mod ast;
pub mod engine;
pub mod incremental;
pub mod parser;
pub mod viewport;
