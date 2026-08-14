//! Движок-парсер Zoll по спецификации `docs/parser.md`.

mod api;
mod collector;
mod dependency;
mod edit;
mod engine;
mod line_map;
mod resolver;
mod simd;

pub use api::*;
pub use collector::{Marker, collect};
pub use dependency::DependencyGraph;
pub use edit::Edit;
pub use engine::*;
pub use line_map::LineMap;
pub use resolver::{SyntaxKind, SyntaxSpan, resolve};
pub use simd::{Event, scan};
