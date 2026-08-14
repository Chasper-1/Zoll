//! Правка документа (разделы 14–16 спеки).
//!
//! Редактор сообщает изменение байтового буфера, а не клавиши:
//! удалить `[position, position + delete_len)` и вставить `insert`.

/// Изменение документа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub position: usize,
    pub delete_len: usize,
    pub insert: Vec<u8>,
}

impl Edit {
    pub fn new(position: usize, delete_len: usize, insert: impl Into<Vec<u8>>) -> Self {
        Edit {
            position,
            delete_len,
            insert: insert.into(),
        }
    }

    /// Изменение длины документа: `insert.len() - delete_len`.
    pub fn delta(&self) -> isize {
        self.insert.len() as isize - self.delete_len as isize
    }

    /// Применить правку к буферу.
    pub fn apply(&self, text: &mut Vec<u8>) {
        let end = self
            .position
            .saturating_add(self.delete_len)
            .min(text.len());
        text.splice(self.position..end, self.insert.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert() {
        let mut text = b"hello world".to_vec();
        Edit::new(5, 0, b",").apply(&mut text);
        assert_eq!(text, b"hello, world");
    }

    #[test]
    fn delete() {
        let mut text = b"hello world".to_vec();
        Edit::new(5, 6, b"").apply(&mut text);
        assert_eq!(text, b"hello");
    }

    #[test]
    fn replace() {
        let mut text = b"abc".to_vec();
        Edit::new(1, 1, b"XYZ").apply(&mut text);
        assert_eq!(text, b"aXYZc");
    }

    #[test]
    fn delta_calc() {
        let e = Edit::new(0, 3, b"12345");
        assert_eq!(e.delta(), 2);
    }
}
