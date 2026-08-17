//! Движок-парсер Zoll по спецификации `docs/parser.md`.

mod api;
mod dependency;
mod edit;
mod line_map;
mod parser;
mod resolver;
mod simd;

pub use api::*;
pub use dependency::DependencyGraph;
pub use line_map::LineMap;
pub use parser::*;
pub use resolver::{SyntaxKind, SyntaxSpan};
pub use simd::scan;
