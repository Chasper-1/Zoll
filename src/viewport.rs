//! Ленивый парсинг: парсер получает от редактора только видимый диапазон строк.
//!
//! `IncrementalDoc::edit_visible()` использует `Viewport`, чтобы перепарсить
//! только видимые строки плюс блок-контейнеры, обеспечивающие корректность AST.

/// Видимый диапазон строк в редакторе.
///
/// Парсер НЕ вычисляет его сам — это задача редактора/лейаута.
/// Парсер только принимает готовые границы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Первая видимая строка (0-based).
    pub first_line: usize,
    /// Последняя видимая строка (0-based, включительно).
    pub last_line: usize,
}

impl Viewport {
    /// Создать viewport по номерам строк.
    ///
    /// Если `last_line < first_line`, строки меняются местами.
    pub fn new(first: usize, last: usize) -> Self {
        if last < first {
            Viewport {
                first_line: last,
                last_line: first,
            }
        } else {
            Viewport {
                first_line: first,
                last_line: last,
            }
        }
    }

    /// Создать viewport по байтовым позициям в тексте.
    ///
    /// Использует `line_starts` для перевода байт → номера строк.
    /// Удобно, когда редактор знает только байтовые границы видимой области.
    pub fn from_bytes(line_starts: &[usize], first_byte: usize, last_byte: usize) -> Self {
        let first = line_number(line_starts, first_byte);
        let last = line_number(line_starts, last_byte);
        Viewport::new(first, last)
    }

    /// Количество видимых строк.
    pub fn height(&self) -> usize {
        if self.last_line >= self.first_line {
            self.last_line - self.first_line + 1
        } else {
            0
        }
    }

    /// Проверить, входит ли строка в видимый диапазон.
    pub fn contains(&self, line: usize) -> bool {
        line >= self.first_line && line <= self.last_line
    }
}

/// Номер строки по байтовой позиции (0-based).
fn line_number(line_starts: &[usize], byte_pos: usize) -> usize {
    match line_starts.binary_search(&byte_pos) {
        Ok(i) => i,
        Err(i) => {
            if i == 0 {
                0
            } else {
                i - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_new_normal() {
        let vp = Viewport::new(3, 7);
        assert_eq!(vp.first_line, 3);
        assert_eq!(vp.last_line, 7);
        assert_eq!(vp.height(), 5);
    }

    #[test]
    fn viewport_new_swapped() {
        let vp = Viewport::new(7, 3);
        assert_eq!(vp.first_line, 3);
        assert_eq!(vp.last_line, 7);
    }

    #[test]
    fn viewport_single_line() {
        let vp = Viewport::new(5, 5);
        assert_eq!(vp.height(), 1);
        assert!(vp.contains(5));
        assert!(!vp.contains(4));
        assert!(!vp.contains(6));
    }

    #[test]
    fn viewport_from_bytes() {
        let line_starts = vec![0, 6, 12, 18, 24];
        let vp = Viewport::from_bytes(&line_starts, 7, 20);
        assert_eq!(vp.first_line, 1);
        assert_eq!(vp.last_line, 3);
    }
}
