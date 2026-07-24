//! Инкрементальный документ с построчным хранением AST.
//!
//! `IncrementalDoc` хранит source + line_asts (по строке).
//! Любая правка перепарсивает только изменённые строки и пересобирает
//! общий AST через merge, начиная с первого затронутого блока.

use crate::ast::{LineAST, MarkupDoc};
use crate::parser::merge;
use crate::viewport::Viewport;

/// Инкрементальный документ.
///
/// # Пример
///
/// ```rust
/// use zoll::incremental::IncrementalDoc;
///
/// let mut doc = IncrementalDoc::new("**hello** world");
/// doc.edit(0, 0, "very ");
/// ```
pub struct IncrementalDoc {
    /// Исходный текст.
    pub source: String,
    /// Байтовые начала строк (line_starts[i] = байт начала строки i).
    pub line_starts: Vec<usize>,
    /// AST каждой строки (после parse_line).
    pub line_asts: Vec<LineAST>,
    /// Собранный общий AST (после merge).
    pub merged_ast: MarkupDoc,
}

impl IncrementalDoc {
    /// Создать новый документ из текста.
    pub fn new(text: &str) -> Self {
        let line_starts = build_line_starts(text);
        let line_asts: Vec<LineAST> = text.lines().map(|l| parse_line_or_empty(l)).collect();
        let merged_ast = merge(&line_asts);

        IncrementalDoc {
            source: text.to_string(),
            line_starts,
            line_asts,
            merged_ast,
        }
    }

    /// Применить правку: удалить `[from..to)` и вставить `text`.
    pub fn edit(&mut self, from: usize, to: usize, text: &str) -> &MarkupDoc {
        let old_lines_before = self.line_at_byte(from);
        let old_lines_removed = if to > from {
            self.line_at_byte(to).saturating_sub(old_lines_before)
        } else {
            0
        };

        self.source.replace_range(from..to, text);
        self.rebuild_line_starts(from);

        let new_lines = self.source[self.line_starts[old_lines_before]..]
            .lines()
            .count()
            .max(1);
        let changed_line_count = new_lines + old_lines_removed;

        let start_line = old_lines_before;
        let end_line = (start_line + changed_line_count).min(self.line_asts.len());

        while self.line_asts.len() < self.line_starts.len() {
            self.line_asts.push(LineAST::Empty);
        }

        for i in start_line..end_line {
            let line = self.get_line_text(i);
            self.line_asts[i] = parse_line_or_empty(&line);
        }

        let expected_lines = self.line_starts.len();
        self.line_asts.truncate(expected_lines);

        while self.line_asts.len() < expected_lines {
            self.line_asts.push(LineAST::Empty);
        }

        let merge_start = self.find_block_start(start_line);
        let partial: Vec<LineAST> = self.line_asts[merge_start..].to_vec();

        if merge_start == 0 {
            self.merged_ast = merge(&self.line_asts);
        } else {
            let clean = merge(&self.line_asts[..merge_start]);
            let dirty = merge(&partial);
            let mut combined = clean;
            combined.children.extend(dirty.children);
            self.merged_ast = combined;
        }

        &self.merged_ast
    }

    /// Применить правку и перепарсить только видимый диапазон + блоки.
    ///
    /// Работает как `edit()`, но merge делает только для строк,
    /// попадающих в `viewport`, плюс блок-контейнеры, в которые они входят.
    /// Строки вне видимости НЕ парсятся заново (используется старый `line_ast`).
    pub fn edit_visible(&mut self, from: usize, to: usize, text: &str, viewport: &Viewport) -> &MarkupDoc {
        // 1. Применяем правку к source
        let old_lines_before = self.line_at_byte(from);
        let old_lines_removed = if to > from {
            self.line_at_byte(to).saturating_sub(old_lines_before)
        } else {
            0
        };

        self.source.replace_range(from..to, text);
        self.rebuild_line_starts(from);

        // 2. Перепарсиваем ТОЛЬКО изменившиеся строки
        let new_lines = self.source[self.line_starts[old_lines_before]..]
            .lines()
            .count()
            .max(1);
        let changed_line_count = new_lines + old_lines_removed;

        let start_line = old_lines_before;
        let end_line = (start_line + changed_line_count).min(self.line_asts.len());

        while self.line_asts.len() < self.line_starts.len() {
            self.line_asts.push(LineAST::Empty);
        }

        for i in start_line..end_line {
            let line = self.get_line_text(i);
            self.line_asts[i] = parse_line_or_empty(&line);
        }

        let expected_lines = self.line_starts.len();
        self.line_asts.truncate(expected_lines);

        while self.line_asts.len() < expected_lines {
            self.line_asts.push(LineAST::Empty);
        }

        // 3. Определяем диапазон для merge: от начала блока, содержащего viewport,
        //    до конца viewport
        let merge_start = self.find_block_start(viewport.first_line.min(start_line));
        let merge_end = (viewport.last_line + 1).min(self.line_asts.len());

        // 4. Merge только видимого диапазона
        if merge_start == 0 {
            let visible = &self.line_asts[..merge_end];
            self.merged_ast = merge(visible);
        } else {
            let clean = merge(&self.line_asts[..merge_start]);
            let visible = &self.line_asts[merge_start..merge_end];
            let dirty = merge(visible);
            let mut combined = clean;
            combined.children.extend(dirty.children);
            self.merged_ast = combined;
        }

        &self.merged_ast
    }

    /// Получить текст строки по индексу.
    fn get_line_text(&self, idx: usize) -> String {
        if idx >= self.line_starts.len() {
            return String::new();
        }
        let start = self.line_starts[idx];
        let end = if idx + 1 < self.line_starts.len() {
            self.line_starts[idx + 1]
        } else {
            self.source.len()
        };
        let mut line = self.source[start..end].to_string();
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        line
    }

    /// Найти номер строки по байтовой позиции.
    pub fn line_number(&self, byte_pos: usize) -> usize {
        let byte_pos = byte_pos.min(self.source.len());
        match self.line_starts.binary_search(&byte_pos) {
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

    /// Количество строк.
    pub fn num_lines(&self) -> usize {
        self.line_starts.len()
    }

    // ─── Приватные помощники ─────────────────────────────────

    fn line_at_byte(&self, byte: usize) -> usize {
        self.line_number(byte)
    }

    fn rebuild_line_starts(&mut self, from: usize) {
        let start_idx = self.line_at_byte(from);

        let mut result: Vec<usize> = self.line_starts[..=start_idx].to_vec();
        result.truncate(start_idx + 1);

        let suffix: Vec<usize> = self.source[from..]
            .char_indices()
            .filter(|(_, c)| *c == '\n')
            .map(|(i, _)| from + i + 1)
            .collect();

        result.extend(suffix);
        self.line_starts = result;
    }

    /// Найти начало блок-левел блока, содержащего `line`.
    fn find_block_start(&self, line: usize) -> usize {
        // Сначала считаем глубину вложенности на строке `line`
        let mut depth = 0i32;
        for i in 0..line {
            match &self.line_asts[i] {
                LineAST::BlockMarker(_) => {
                    if depth > 0 { depth -= 1; } else { depth += 1; }
                }
                LineAST::SpoilerBlockOpen(_) => { depth += 1; }
                _ => {}
            }
        }
        // Если строка НЕ внутри блока — начинаем с неё
        if depth <= 0 { return line; }
        // Строка внутри блока — идём назад и ищем открывающий маркер
        let mut close_depth = depth;
        let mut i = line;
        while i > 0 {
            i -= 1;
            match &self.line_asts[i] {
                LineAST::BlockMarker(_) | LineAST::SpoilerBlockOpen(_) => {
                    if close_depth <= 1 { return i; }
                    close_depth -= 1;
                }
                _ => {}
            }
        }
        0
    }
}

// ─── Помощники ───────────────────────────────────────────────

/// Парсит строку или возвращает Empty для пустой.
fn parse_line_or_empty(line: &str) -> LineAST {
    if line.trim().is_empty() && line.is_empty() {
        if line.is_empty() {
            return LineAST::Empty;
        }
    }
    crate::parser::parse_line(line)
}

/// Построить массив начал строк из текста.
pub fn build_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, c) in text.char_indices() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

// ─── Тесты ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::MarkupNode;
    use crate::ast::MarkupStyle;

    #[test]
    fn new_doc_creates_lines() {
        let doc = IncrementalDoc::new("hello\nworld");
        assert_eq!(doc.line_starts.len(), 2);
        assert_eq!(doc.line_asts.len(), 2);
    }

    #[test]
    fn edit_single_line() {
        let mut doc = IncrementalDoc::new("hello world");
        doc.edit(0, 5, "hi");
        assert_eq!(doc.source, "hi world");
    }

    #[test]
    fn edit_preserves_ast() {
        let mut doc = IncrementalDoc::new("**bold** text");
        doc.edit(9, 13, "content");
        let has_bold = doc.merged_ast.children.iter().any(|n| {
            matches!(n, MarkupNode::Formatted { style, .. } if *style == MarkupStyle::BOLD)
        });
        assert!(has_bold, "Bold formatting should be preserved after edit");
    }

    #[test]
    fn edit_preserves_line_count() {
        let mut doc = IncrementalDoc::new("line1\nline2\nline3");
        assert_eq!(doc.num_lines(), 3);
        doc.edit(0, 0, "X");
        assert_eq!(doc.num_lines(), 3);
    }

    #[test]
    fn edit_adds_newlines() {
        let mut doc = IncrementalDoc::new("hello world");
        doc.edit(6, 6, "\nnew\nlines\n");
        assert!(doc.num_lines() >= 3, "should have at least 3 lines, got {}", doc.num_lines());
        assert_eq!(doc.source, "hello \nnew\nlines\nworld");
    }

    #[test]
    fn edit_removes_lines() {
        let mut doc = IncrementalDoc::new("a\nb\nc\nd");
        doc.edit(2, 5, "");
        assert_eq!(doc.source, "a\n\nd");
    }

    #[test]
    fn simple_text_parse() {
        let doc = IncrementalDoc::new("hello world");
        assert_eq!(doc.merged_ast.children.len(), 1);
    }

    #[test]
    fn empty_source() {
        let doc = IncrementalDoc::new("");
        assert_eq!(doc.line_starts.len(), 1);
        assert_eq!(doc.merged_ast.children.len(), 0);
    }

    #[test]
    fn header_in_doc() {
        let doc = IncrementalDoc::new("#1# Title\ncontent");
        assert_eq!(doc.merged_ast.children.len(), 2);
        assert!(matches!(&doc.merged_ast.children[0], MarkupNode::Header { level: 1, .. }));
    }

    #[test]
    fn multiline_paragraph() {
        let doc = IncrementalDoc::new("line1\nline2\n\nline3");
        assert!(doc.merged_ast.children.len() >= 2);
    }

    #[test]
    fn edit_visible_only_parses_viewport() {
        let mut doc = IncrementalDoc::new(
            "%%%\n\
             hidden\n\
             %%%\n\
             visible **bold** text\n\
             more visible\n\
             hidden2\n\
             hidden3"
        );
        let viewport = Viewport { first_line: 3, last_line: 4 };
        doc.edit_visible(15, 15, "X", &viewport);
        // Строка с bold изменилась, bold должен сохраниться
        let has_bold = doc.merged_ast.children.iter().any(|n| {
            matches!(n, MarkupNode::Formatted { style, .. } if *style == MarkupStyle::BOLD)
        });
        assert!(has_bold, "Bold should be preserved in visible area");
    }
}
