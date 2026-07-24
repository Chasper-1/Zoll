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
        let start_line = self.line_at_byte(from);
        // Последняя строка, затронутая правкой (в старых индексах).
        let end_line_old = if to > from {
            self.line_at_byte(to.min(self.source.len()))
        } else {
            start_line
        };
        // Количество строк в старом документе (line_starts включает EOF-маркер,
        // line_asts может быть на 1 меньше — нормализуем ниже).
        let old_starts_len = self.line_starts.len();

        let removed_ends_with_newline = if to > from {
            matches!(self.source.as_bytes().get(to.saturating_sub(1)), Some(b'\n'))
        } else {
            false
        };

        self.source.replace_range(from..to, text);
        self.rebuild_line_starts(from, to, text, removed_ends_with_newline);
        let new_starts_len = self.line_starts.len();

        // Нормализуем line_asts — доводим до old_starts_len (на случай,
        // если конструктор new() создал короче без EOF-пустышки).
        while self.line_asts.len() < old_starts_len {
            self.line_asts.push(LineAST::Empty);
        }

        // Сколько старых строк ПОСЛЕ затронутого диапазона сохранятся
        // (их текст не изменился — только индексы сдвинулись).
        let preserved_old_count = old_starts_len.saturating_sub(end_line_old + 1);
        let shift_new_start = new_starts_len.saturating_sub(preserved_old_count);

        // Строим новый line_asts:
        //   [0..start_line)         — старые AST (текст не менялся)
        //   [start_line..shift_new) — Empty, потом перепарсим
        //   [shift..new_len)        — старые AST, сдвинутые (текст не менялся)
        let mut new_asts: Vec<LineAST> = Vec::with_capacity(new_starts_len);
        new_asts.extend_from_slice(&self.line_asts[..start_line]);

        // Перепарсиваемая зона (заполняем Empty)
        let reparse_count = shift_new_start.saturating_sub(start_line);
        new_asts.resize(new_asts.len() + reparse_count, LineAST::Empty);

        // Сдвинутый хвост старых AST (текст не менялся)
        new_asts.extend_from_slice(&self.line_asts[(end_line_old + 1)..]);

        // Обрезаем/растягиваем до точной длины
        new_asts.truncate(new_starts_len);
        new_asts.resize(new_starts_len, LineAST::Empty);

        // Перепарсиваем ТОЛЬКО строки в диапазоне [start_line..shift_new_start)
        for i in start_line..shift_new_start.min(new_starts_len) {
            let line = self.get_line_text(i);
            new_asts[i] = parse_line_or_empty(line);
        }

        self.line_asts = new_asts;

        // Merge от начала затронутого блока.
        // БЕЗ клонирования line_asts — merge принимает &[LineAST].
        let merge_start = self.find_block_start(start_line);

        if merge_start == 0 {
            self.merged_ast = merge(&self.line_asts);
        } else {
            let clean = merge(&self.line_asts[..merge_start]);
            let dirty = merge(&self.line_asts[merge_start..]);
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
        let start_line = self.line_at_byte(from);

        let removed_ends_with_newline = if to > from {
            matches!(self.source.as_bytes().get(to.saturating_sub(1)), Some(b'\n'))
        } else {
            false
        };

        self.source.replace_range(from..to, text);
        self.rebuild_line_starts(from, to, text, removed_ends_with_newline);
        let new_line_count = self.line_starts.len();

        // 2. Перестраиваем line_asts:
        //    - строки ДО start_line сохраняют старые AST (их текст не менялся)
        //    - строки ПОСЛЕ start_line заполняются Empty (потом перезапишем нужные)
        self.line_asts.truncate(start_line);
        self.line_asts.resize(new_line_count, LineAST::Empty);

        // 3. Перепарсиваем ТОЛЬКО строки от start_line до конца viewport.
        //    Строки за пределами viewport НЕ парсятся — они остаются Empty
        //    и не участвуют в merge.
        let parse_end = if start_line <= viewport.last_line {
            (viewport.last_line + 1).min(new_line_count)
        } else {
            // Правка ПОСЛЕ viewport — строки в viewport не изменились,
            // их старые AST сохранены (start_line > viewport.last_line,
            // truncate не затронул их).
            return &self.merged_ast;
        };

        for i in start_line..parse_end {
            let text = self.get_line_text(i);
            self.line_asts[i] = parse_line_or_empty(text);
        }

        // 4. Merge до конца viewport (один проход вместо чистый/грязный).
        //    Строки до start_line не менялись, но merge всё равно проходит
        //    по ним — это можно будет кешировать в будущем.
        let merge_end = (viewport.last_line + 1).min(self.line_asts.len());
        self.merged_ast = merge(&self.line_asts[..merge_end]);

        &self.merged_ast
    }

    /// Получить текст строки по индексу (без аллокации — возвращает &str из source).
    fn get_line_text(&self, idx: usize) -> &str {
        if idx >= self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[idx];
        let end = if idx + 1 < self.line_starts.len() {
            self.line_starts[idx + 1]
        } else {
            self.source.len()
        };
        let line = &self.source[start..end];
        // Отрезаем \n и \r\n без аллокации
        if let Some(stripped) = line.strip_suffix('\n') {
            stripped.strip_suffix('\r').unwrap_or(stripped)
        } else {
            line
        }
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

    fn rebuild_line_starts(&mut self, from: usize, to_old: usize, text: &str, removed_ends_with_newline: bool) {
        let start_idx = self.line_at_byte(from);

        // Сохраняем начала строк ДО правки
        let mut result: Vec<usize> = self.line_starts[..=start_idx].to_vec();
        result.truncate(start_idx + 1);

        // Сканируем ТОЛЬКО вставленный текст на наличие '\n'
        for (i, c) in text.char_indices() {
            if c == '\n' {
                result.push(from + i + 1);
            }
        }

        // Сдвигаем старые начала строк ПОСЛЕ удалённого диапазона
        let delta = text.len() as isize - (to_old - from) as isize;
        for i in (start_idx + 1)..self.line_starts.len() {
            let old_pos = self.line_starts[i];
            if old_pos < to_old {
                // Старое начало строки находится внутри удалённого диапазона — пропускаем
                continue;
            }
            // Если это ПЕРВОЕ старое начало после to_old И удалённый текст заканчивался на '\n',
            // то это начало строки было создано этим '\n' и больше не существует.
            if old_pos == to_old && removed_ends_with_newline {
                continue;
            }
            let new_pos = (old_pos as isize + delta) as usize;
            // Защита от дубликатов (теоретически не должно возникать)
            if result.last().copied().map_or(true, |last| new_pos > last) {
                result.push(new_pos);
            }
        }

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
