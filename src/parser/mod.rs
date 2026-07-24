//! Строчный парсер zoll.
//!
//! - `parse_line()` — парсит одну строку в `LineAST`
//! - `merge()` — собирает `Vec<LineAST>` в `MarkupDoc`

mod line;
mod merge;

pub use line::parse_line;
pub use merge::{merge, parse_full};
