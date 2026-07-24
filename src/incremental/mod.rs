//! Инкрементальный парсер zoll.
//!
//! Позволяет редактировать текст и перепаривать только изменившийся диапазон,
//! а не весь документ целиком.
//!
//! # Пример
//!
//! ```rust
//! use zoll::incremental::IncrementalDoc;
//!
//! let mut doc = IncrementalDoc::new("**hello** world");
//! doc.edit(0, 0, "very ");  // вставить "very " в начало
//! // Теперь AST обновлён
//! ```

mod doc;

pub use doc::{build_line_starts, IncrementalDoc};
