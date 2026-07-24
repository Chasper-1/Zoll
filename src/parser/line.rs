//! Парсер одной строки zoll.
//!
//! Определяет тип строки по первой колонке и парсит inline-маркеры.

use crate::ast::{BlockType, LineAST, MarkupNode, MarkupStyle};

/// Парсит одну строку (без `\n`) в `LineAST`.
pub fn parse_line(line: &str) -> LineAST {
    let trimmed = line.trim_start();

    // ── Пустая строка ──
    if trimmed.is_empty() {
        return LineAST::Empty;
    }

    // ── Block-level: %%% / $$$ / !!! ──
    if let Some(rest) = trimmed.strip_prefix("%%%") {
        if rest.trim().is_empty() {
            return LineAST::BlockMarker(BlockType::Comment);
        }
        return LineAST::BlockMarker(BlockType::Comment);
    }
    if let Some(rest) = trimmed.strip_prefix("$$$") {
        if rest.trim().is_empty() {
            return LineAST::BlockMarker(BlockType::Formula);
        }
        return LineAST::BlockMarker(BlockType::Formula);
    }
    if let Some(rest) = trimmed.strip_prefix("!!!") {
        if rest.trim().is_empty() {
            return LineAST::BlockMarker(BlockType::Spoiler);
        }
        let spoiler_rest = rest.trim();
        if let Some(title_end) = spoiler_rest.find(':') {
            let title = spoiler_rest[..title_end].trim().to_string();
            if title.is_empty() {
                return LineAST::SpoilerBlockOpen(None);
            }
            return LineAST::SpoilerBlockOpen(Some(title));
        }
        return LineAST::BlockMarker(BlockType::Spoiler);
    }

    // ── Line-level: %% (комментарий до конца строки с любого места) ──
    if let Some(pos) = trimmed.find("%%") {
        let before = &trimmed[..pos];
        let after = &trimmed[pos + 2..];
        let _children = parse_inline(before.trim_end());
        let comment_content = parse_inline(after.trim());
        return LineAST::Comment(comment_content);
    }

    // ── $$ с любого места строки ──
    if let Some(pos) = trimmed.find("$$") {
        let _before = &trimmed[..pos];
        let after = &trimmed[pos + 2..];
        let children = parse_inline(after.trim());
        return LineAST::Formula(children);
    }

    // ── !! с любого места строки ── (не !!!, проверено выше)
    if let Some(pos) = trimmed.find("!!") {
        let _before = &trimmed[..pos];
        let after = &trimmed[pos + 2..];
        let rest = after.trim();
        if let Some(title_end) = rest.find(':') {
            let title = rest[..title_end].trim().to_string();
            let content = rest[title_end + 1..].trim();
            return LineAST::Spoiler(Some(title), parse_inline(content));
        }
        return LineAST::Spoiler(None, parse_inline(rest));
    }

    // ── #N# Заголовок ──
    if let Some(rest) = trimmed.strip_prefix('#') {
        let level_end = rest.find('#');
        if let Some(end) = level_end {
            if end > 0 {
                let level_str = &rest[..end];
                if let Ok(level) = level_str.parse::<u32>() {
                    let content = rest[end + 1..].trim();
                    return LineAST::Header(level, parse_inline(content));
                }
            }
        }
    }

    // ── ThematicBreak: --- / ___ / *** ──
    let pure = trimmed.trim();
    if pure == "---" || pure == "___" || pure == "***" {
        return LineAST::ThematicBreak;
    }

    // ── Ненумерованный список: - / * / + с пробелом ПОСЛЕ маркера ──
    for delim in &['-', '*', '+'] {
        if let Some(rest) = trimmed.strip_prefix(*delim) {
            if rest.is_empty() || rest.starts_with(' ') {
                let content = rest.trim();
                return LineAST::ListItem(false, 0, parse_inline(content));
            }
        }
    }

    // ── Нумерованный список: 1. / 2. / ... ──
    if let Some(end) = trimmed.find(|c: char| !c.is_ascii_digit()) {
        if end > 0 && trimmed.as_bytes().get(end) == Some(&b'.') {
            let num = trimmed[..end].parse::<u32>().unwrap_or(1);
            let content = trimmed[end + 1..].trim();
            if trimmed.as_bytes().get(end + 1).map_or(true, |&b| b == b' ') {
                return LineAST::ListItem(true, num, parse_inline(content));
            }
        }
    }

    // ── Цитата: > ──
    if let Some(rest) = trimmed.strip_prefix('>') {
        let content = rest.trim();
        return LineAST::Quote(parse_inline(content));
    }

    // ── Строка таблицы: | ... | ──
    if trimmed.starts_with('|') {
        return parse_table_row(trimmed);
    }

    // ── Тэг: #:tag ──
    if let Some(rest) = trimmed.strip_prefix("#:") {
        return LineAST::Tag(rest.trim().to_string());
    }

    // ── Ничего не подошло → обычный параграф ──
    LineAST::Paragraph(parse_inline(trimmed))
}

/// Парсит inline-маркеры в тексте.
///
/// Поддерживает: ** // __ '' ,, ~~ == ++ -- $ $
pub fn parse_inline(text: &str) -> Vec<MarkupNode> {
    let bytes = text.as_bytes();
    let len = text.len();
    let mut pos = 0;
    let mut nodes: Vec<MarkupNode> = Vec::new();
    let mut text_start: Option<usize> = None;

    while pos < len {
        let b = bytes[pos];

        // ── Экранирование ──
        if b == b'\\' && pos + 1 < len {
            flush_text(&mut nodes, text, &mut text_start, pos);
            let next = bytes[pos + 1];
            let ch_len = utf8_char_len(next);
            nodes.push(MarkupNode::Text(text[pos + 1..pos + 1 + ch_len].to_string()));
            pos += 1 + ch_len;
            continue;
        }

        // ── Поиск inline-маркеров ──
        if let Some(marker) = match_inline_marker(bytes, pos, len) {
            let (close_pos, style) = marker;
            let open_len = marker_len(style);
            let open_end = pos + open_len;
            if close_pos > open_end {
                flush_text(&mut nodes, text, &mut text_start, pos);
                let inner_text = &text[open_end..close_pos];
                let inner = parse_inline(inner_text);
                nodes.push(MarkupNode::Formatted {
                    style,
                    children: inner,
                });
                pos = close_pos + open_len;
                continue;
            }
        }

        // ── Накопление текста ──
        if text_start.is_none() {
            text_start = Some(pos);
        }
        pos += 1;
    }

    flush_text(&mut nodes, text, &mut text_start, pos);
    nodes
}

// ─── Таблицы ───────────────────────────────────────────────────

fn parse_table_row(line: &str) -> LineAST {
    let trimmed = line.trim();
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') {
        &trimmed[1..trimmed.len() - 1]
    } else if trimmed.starts_with('|') {
        &trimmed[1..]
    } else {
        trimmed
    };

    let mut cells = Vec::new();
    for cell in inner.split('|') {
        let cell = cell.trim();
        // Обрезаем : для выравнивания
        let cell_text = cell
            .strip_prefix(':')
            .unwrap_or(cell)
            .strip_suffix(':')
            .unwrap_or(cell);
        cells.push(parse_inline(cell_text.trim()));
    }

    LineAST::TableRow(cells)
}

// ─── Помощники inline-парсера ──────────────────────────────────

fn marker_len(style: MarkupStyle) -> usize {
    match style {
        s if s == MarkupStyle::FORMULA => 1,
        _ => 2,
    }
}

fn utf8_char_len(b: u8) -> usize {
    if b < 128 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

fn flush_text(
    nodes: &mut Vec<MarkupNode>,
    text: &str,
    start: &mut Option<usize>,
    end: usize,
) {
    if let Some(s) = start.take() {
        if end > s {
            nodes.push(MarkupNode::Text(text[s..end].to_string()));
        }
    }
}

fn match_inline_marker(
    bytes: &[u8],
    pos: usize,
    len: usize,
) -> Option<(usize, MarkupStyle)> {
    if pos + 1 >= len {
        return None;
    }
    let (style, open_len) = match (bytes[pos], bytes[pos + 1]) {
        (b'*', b'*') => (MarkupStyle::BOLD, 2),
        (b'/', b'/') => (MarkupStyle::ITALIC, 2),
        (b'_', b'_') => (MarkupStyle::UNDERLINE, 2),
        (b'~', b'~') => (MarkupStyle::STRIKETHROUGH, 2),
        (b'=', b'=') => (MarkupStyle::HIGHLIGHT, 2),
        (b'+', b'+') => (MarkupStyle::INSERTION, 2),
        (b'-', b'-') => (MarkupStyle::DELETION, 2),
        (b'\'', b'\'') => (MarkupStyle::SUPERSCRIPT, 2),
        (b',', b',') => (MarkupStyle::SUBSCRIPT, 2),
        (b'$', b'$') => (MarkupStyle::DISPLAY_FORMULA, 2),
        (b'$', _) => return find_close_for_single(bytes, pos, len),
        _ => return None,
    };
    find_close(bytes, pos, open_len, style)
}

fn find_close(
    bytes: &[u8],
    open_pos: usize,
    open_len: usize,
    style: MarkupStyle,
) -> Option<(usize, MarkupStyle)> {
    let start = open_pos + open_len;
    let mut i = start;
    while i + open_len <= bytes.len() {
        if bytes[i] == bytes[open_pos] && bytes[i + 1] == bytes[open_pos + 1] {
            return Some((i, style));
        }
        i += 1;
    }
    None
}

fn find_close_for_single(
    bytes: &[u8],
    open_pos: usize,
    len: usize,
) -> Option<(usize, MarkupStyle)> {
    let start = open_pos + 1;
    for i in start..len {
        if bytes[i] == b'$' {
            return Some((i, MarkupStyle::FORMULA));
        }
    }
    None
}

// ─── Тесты ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text() {
        let ast = parse_line("hello world");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![MarkupNode::Text("hello world".to_string())])
        );
    }

    #[test]
    fn bold_text() {
        let ast = parse_line("**hello**");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![MarkupNode::Formatted {
                style: MarkupStyle::BOLD,
                children: vec![MarkupNode::Text("hello".to_string())],
            }])
        );
    }

    #[test]
    fn italic_text() {
        let ast = parse_line("//hello//");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![MarkupNode::Formatted {
                style: MarkupStyle::ITALIC,
                children: vec![MarkupNode::Text("hello".to_string())],
            }])
        );
    }

    #[test]
    fn nested_formatting() {
        let ast = parse_line("**//bold italic//**");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![MarkupNode::Formatted {
                style: MarkupStyle::BOLD,
                children: vec![MarkupNode::Formatted {
                    style: MarkupStyle::ITALIC,
                    children: vec![MarkupNode::Text("bold italic".to_string())],
                }],
            }])
        );
    }

    #[test]
    fn mixed_text_and_formatting() {
        let ast = parse_line("hello **world**");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![
                MarkupNode::Text("hello ".to_string()),
                MarkupNode::Formatted {
                    style: MarkupStyle::BOLD,
                    children: vec![MarkupNode::Text("world".to_string())],
                },
            ])
        );
    }

    #[test]
    fn header() {
        let ast = parse_line("#1# Title");
        assert_eq!(ast, LineAST::Header(1, vec![MarkupNode::Text("Title".to_string())]));
    }

    #[test]
    fn header_level_3() {
        let ast = parse_line("#3# Sub Section");
        assert_eq!(ast, LineAST::Header(3, vec![MarkupNode::Text("Sub Section".to_string())]));
    }

    #[test]
    fn comment_line() {
        let ast = parse_line("%% this is hidden");
        assert_eq!(
            ast,
            LineAST::Comment(vec![MarkupNode::Text("this is hidden".to_string())])
        );
    }

    #[test]
    fn comment_mid_line() {
        let ast = parse_line("visible %% hidden");
        assert_eq!(
            ast,
            LineAST::Comment(vec![MarkupNode::Text("hidden".to_string())])
        );
    }

    #[test]
    fn formula_line() {
        let ast = parse_line("$$ x = 5");
        assert_eq!(ast, LineAST::Formula(vec![MarkupNode::Text("x = 5".to_string())]));
    }

    #[test]
    fn spoiler_line() {
        let ast = parse_line("!! hidden content");
        assert_eq!(
            ast,
            LineAST::Spoiler(None, vec![MarkupNode::Text("hidden content".to_string())])
        );
    }

    #[test]
    fn spoiler_with_title() {
        let ast = parse_line("!!title: hidden");
        assert_eq!(
            ast,
            LineAST::Spoiler(
                Some("title".to_string()),
                vec![MarkupNode::Text("hidden".to_string())]
            )
        );
    }

    #[test]
    fn thematic_break() {
        assert_eq!(parse_line("---"), LineAST::ThematicBreak);
        assert_eq!(parse_line("___"), LineAST::ThematicBreak);
        assert_eq!(parse_line("***"), LineAST::ThematicBreak);
    }

    #[test]
    fn unordered_list() {
        let ast = parse_line("- item");
        assert_eq!(ast, LineAST::ListItem(false, 0, vec![MarkupNode::Text("item".to_string())]));
    }

    #[test]
    fn unordered_list_with_star() {
        let ast = parse_line("* item");
        assert_eq!(ast, LineAST::ListItem(false, 0, vec![MarkupNode::Text("item".to_string())]));
    }

    #[test]
    fn ordered_list() {
        let ast = parse_line("1. first");
        assert_eq!(ast, LineAST::ListItem(true, 1, vec![MarkupNode::Text("first".to_string())]));
    }

    #[test]
    fn star_not_confused_with_bold() {
        let ast = parse_line("**bold**");
        assert!(matches!(ast, LineAST::Paragraph(_)));
    }

    #[test]
    fn block_marker_comment() {
        assert_eq!(parse_line("%%%"), LineAST::BlockMarker(BlockType::Comment));
    }

    #[test]
    fn block_marker_formula() {
        assert_eq!(parse_line("$$$"), LineAST::BlockMarker(BlockType::Formula));
    }

    #[test]
    fn block_marker_spoiler() {
        assert_eq!(parse_line("!!!"), LineAST::BlockMarker(BlockType::Spoiler));
    }

    #[test]
    fn quote() {
        let ast = parse_line("> quoted text");
        assert_eq!(ast, LineAST::Quote(vec![MarkupNode::Text("quoted text".to_string())]));
    }

    #[test]
    fn table_row() {
        let ast = parse_line("| a | b | c |");
        if let LineAST::TableRow(cells) = ast {
            assert_eq!(cells.len(), 3);
            assert_eq!(cells[0], vec![MarkupNode::Text("a".to_string())]);
            assert_eq!(cells[1], vec![MarkupNode::Text("b".to_string())]);
            assert_eq!(cells[2], vec![MarkupNode::Text("c".to_string())]);
        } else {
            panic!("expected TableRow");
        }
    }

    #[test]
    fn empty_line() {
        assert_eq!(parse_line(""), LineAST::Empty);
        assert_eq!(parse_line("   "), LineAST::Empty);
    }

    #[test]
    fn escape_char() {
        let ast = parse_line(r"\**not bold**");
        assert_eq!(
            ast,
            LineAST::Paragraph(vec![
                MarkupNode::Text("*".to_string()),
                MarkupNode::Text("*not bold**".to_string()),
            ])
        );
    }

    #[test]
    fn spoiler_block_open_with_title() {
        let ast = parse_line("!!!spoiler block:");
        assert_eq!(ast, LineAST::SpoilerBlockOpen(Some("spoiler block".to_string())));
    }

    #[test]
    fn mixed_on_line() {
        let ast = parse_line("text %% comment");
        assert_eq!(
            ast,
            LineAST::Comment(vec![MarkupNode::Text("comment".to_string())])
        );
    }
}
