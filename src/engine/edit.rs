//! Правка документа (разделы 14–16 спеки).
//!
//! Внутреннее представление изменения байтового буфера. Публичный
//! интерфейс для редактора — отдельные ручки вставки/удаления/замены
//! (`EngineHandle::insert` / `delete` / `replace`), чтобы не думать,
//! что и куда: вставить — значит вставить, удалить — значит удалить.

// Изменение документа: удалить `[position, position + delete_len)`
// и вставить `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub position: usize,
    pub delete_len: usize,
    pub insert: Vec<u8>,
}

impl Edit {
    pub(crate) fn new(position: usize, delete_len: usize, insert: impl Into<Vec<u8>>) -> Self {
        Edit {
            position,
            delete_len,
            insert: insert.into(),
        }
    }

    // Применить правку к буферу.
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
}
