//! Карта строк: строится из событий `\n` первичного потока.
//!
//! Позволяет мгновенно перевести byte offset → номер строки и границы строки.

use crate::engine::simd::Event;

/// Карта строк документа.
#[derive(Debug, Clone, Default)]
pub struct LineMap {
    /// Абсолютные позиции символов `\n` (по возрастанию).
    pub newline_positions: Vec<usize>,
}

impl LineMap {
    /// Построить карту из событий первичного потока.
    pub fn from_events(events: &[Event]) -> Self {
        let newline_positions = events
            .iter()
            .filter(|e| e.byte == b'\n')
            .map(|e| e.position)
            .collect();
        LineMap { newline_positions }
    }

    /// Номер строки (0-based), содержащей байт `byte`.
    pub fn line_at(&self, byte: usize) -> usize {
        match self.newline_positions.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i,
        }
    }

    /// Границы строки `line`: `(start, end)` — полуинтервал `[start, end)`.
    ///
    /// `end` — позиция `\n` (или `usize::MAX` для последней строки без `\n`).
    pub fn line_bounds(&self, line: usize) -> (usize, usize) {
        let start = if line == 0 {
            0
        } else {
            self.newline_positions[line - 1] + 1
        };
        let end = self
            .newline_positions
            .get(line)
            .copied()
            .unwrap_or(usize::MAX);
        (start, end)
    }

    /// Количество строк.
    pub fn num_lines(&self) -> usize {
        self.newline_positions.len() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(text: &[u8]) -> LineMap {
        let events: Vec<Event> = text
            .iter()
            .enumerate()
            .filter(|&(_, &byte)| byte == b'\n')
            .map(|(i, _)| Event {
                position: i,
                byte: b'\n',
            })
            .collect();
        LineMap::from_events(&events)
    }

    #[test]
    fn line_at_basic() {
        let line_map = map(b"abc\ndef\nghi");
        assert_eq!(line_map.line_at(0), 0);
        assert_eq!(line_map.line_at(4), 1);
        assert_eq!(line_map.line_at(8), 2);
    }

    #[test]
    fn line_bounds_basic() {
        let line_map = map(b"abc\ndef\nghi");
        assert_eq!(line_map.line_bounds(0), (0, 3));
        assert_eq!(line_map.line_bounds(1), (4, 7));
        assert_eq!(line_map.line_bounds(2), (8, usize::MAX));
    }

    #[test]
    fn num_lines() {
        let line_map = map(b"a\nb\nc");
        assert_eq!(line_map.num_lines(), 3);
    }

    #[test]
    fn empty() {
        let line_map = map(b"");
        assert_eq!(line_map.num_lines(), 1);
        assert_eq!(line_map.line_bounds(0), (0, usize::MAX));
    }
}
