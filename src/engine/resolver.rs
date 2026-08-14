//! Классификация маркеров: превращает маркеры-последовательности в
//! синтаксические диапазоны (`SyntaxSpan { start, end, kind }`) в байтовых
//! координатах.
//!
//! Вызывается из одного прохода парсинга (`parser::parse_document`) на
//! каждый завершённый маркер — отдельного этапа `collect`/`resolve` нет.
//!
//! ## Inline-маркеры
//!
//! Стек открытых маркеров. Каждый открытый маркер применяет своё свойство
//! от своей позиции до закрывашки. `))` закрывает **все** открытые inline
//! маркеры разом — свойства наслаиваются на один и тот же текст:
//!
//! ```text
//! **//текст))
//! └─bold────┘   bold от `**` до закрывашки
//!   └─italic┘   italic от `//` до закрывашки
//! ```
//!
//! `))` без открытых маркеров выбрасывается. На `\n` незакрытые inline
//! маркеры сбрасываются (inline не выходит за пределы строки).
//!
//! ## Line-маркеры
//!
//! Проверяются только в начале строки (контекст строки). Закрытие —
//! явным `}` или автоматически на `\n`.

// Вид синтаксической конструкции.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    // Inline
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Highlight,
    Insertion,
    Deletion,
    Superscript,
    Subscript,
    Formula,
    // Line
    Header(u32),
    Comment,
    Spoiler,
    Quote,
    ListItem,
    TableRow,
    Tag,
    ThematicBreak,
}

// Синтаксический диапазон в байтовых координатах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start: usize,
    pub end: usize,
    pub kind: SyntaxKind,
}

// Состояние однопроходного разбора: текст, текущая строка, стеки и спаны.
pub(crate) struct ResolveState<'a> {
    pub text: &'a [u8],
    pub line_start: usize,
    pub spans: Vec<SyntaxSpan>,
    pub inline_stack: Vec<(SyntaxKind, usize)>,
    pub line_stack: Vec<(SyntaxKind, usize)>,
}

impl<'a> ResolveState<'a> {
    pub(crate) fn new(text: &'a [u8]) -> Self {
        ResolveState {
            text,
            line_start: 0,
            spans: Vec::new(),
            inline_stack: Vec::new(),
            line_stack: Vec::new(),
        }
    }
}

// Разбирает завершённый маркер `byte` длины `len`, начинающийся в `start`.
//
// Может не открыть/закрыть ничего: маркер просто игнорируется.
pub(crate) fn process_marker(state: &mut ResolveState<'_>, byte: u8, start: usize, len: usize) {
    let text = state.text;
    let end = start + len;
    match byte {
        b')' if len >= 2 => {
            // Универсальная inline-закрывашка.
            if state.inline_stack.is_empty() {
                return; // нет открытого состояния — не закрывашка
            }
            // Правило пробелов: перед `))` не должно быть пробела.
            if start > 0 && text[start - 1] == b' ' {
                return;
            }
            while let Some((kind, open_start)) = state.inline_stack.pop() {
                state.spans.push(SyntaxSpan {
                    start: open_start,
                    end,
                    kind,
                });
            }
        }
        b'}' => {
            // Контекстная line-level закрывашка: только при открытом состоянии.
            if state.line_stack.is_empty() {
                return;
            }
            // Правило пробелов: перед `}` не должно быть пробела.
            if start > 0 && text[start - 1] == b' ' {
                return;
            }
            if let Some((kind, open_start)) = state.line_stack.pop() {
                state.spans.push(SyntaxSpan {
                    start: open_start,
                    end,
                    kind,
                });
            }
        }
        b'.' => {
            // Нумерованный список `1. ` — цифры от начала строки, затем пробел.
            if let Some(span) = try_numbered_list(text, start, state.line_start) {
                state.spans.push(span);
            }
        }
        _ => {
            // Line-level открытие — с любого места строки.
            if let Some(kind) = line_level_open(byte, len) {
                // Правило пробелов: после маркера не должно быть пробела.
                if end < text.len() && text[end] == b' ' {
                    return;
                }
                state.line_stack.push((kind, start));
                return;
            }
            // Line-маркеры — только в начале строки. Проверка «начало строки»
            // дорогая (сканирование префикса), поэтому выполняется только для
            // байтов, которые вообще могут быть line-маркерами.
            if is_line_marker_byte(byte)
                && is_line_start(start, state.line_start, text)
                && let Some(span) = try_line_marker(text, byte, start, len, state.line_start)
            {
                state.spans.push(span);
                return;
            }
            // Inline-открытие.
            if let Some(kind) = inline_open(byte, len) {
                // Правило пробелов: после открывашки не должно быть пробела.
                if end < text.len() && text[end] == b' ' {
                    return;
                }
                state.inline_stack.push((kind, start));
            }
        }
    }
}

// Может ли байт быть line-маркером (заголовок/тег/цитата/список/таблица).
fn is_line_marker_byte(byte: u8) -> bool {
    matches!(byte, b'#' | b'>' | b'|' | b'*' | b'-' | b'+')
}

// `true`, если байт `pos` — первая не-пробельная позиция своей строки.
fn is_line_start(pos: usize, line_start: usize, text: &[u8]) -> bool {
    pos == line_start
        || text[line_start..pos]
            .iter()
            .all(|&byte| byte == b' ' || byte == b'\t')
}

// Свойство inline-маркера по байту и длине последовательности.
fn inline_open(byte: u8, len: usize) -> Option<SyntaxKind> {
    match (byte, len) {
        (b'*', 2) => Some(SyntaxKind::Bold),
        (b'/', 2) => Some(SyntaxKind::Italic),
        (b'_', 2) => Some(SyntaxKind::Underline),
        (b'~', 2) => Some(SyntaxKind::Strikethrough),
        (b'=', 2) => Some(SyntaxKind::Highlight),
        (b'+', 2) => Some(SyntaxKind::Insertion),
        (b'-', 2) => Some(SyntaxKind::Deletion),
        (b'\'', 2) => Some(SyntaxKind::Superscript),
        (b',', 2) => Some(SyntaxKind::Subscript),
        (b'$', 1) => Some(SyntaxKind::Formula),
        _ => None,
    }
}

// Свойство line-level маркера (%%/$$/!!) — закрывается `}`.
fn line_level_open(byte: u8, len: usize) -> Option<SyntaxKind> {
    match (byte, len) {
        (b'%', 2) => Some(SyntaxKind::Comment),
        (b'$', 2) => Some(SyntaxKind::Formula),
        (b'!', 2) => Some(SyntaxKind::Spoiler),
        _ => None,
    }
}

// Позиция конца строки: до следующего `\n` или конца текста.
fn line_end_of(text: &[u8], from: usize) -> usize {
    text[from..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map(|rel| from + rel)
        .unwrap_or(text.len())
}

// Line-маркер в начале строки.
fn try_line_marker(
    text: &[u8],
    byte: u8,
    start: usize,
    len: usize,
    line_start: usize,
) -> Option<SyntaxSpan> {
    let line_end = line_end_of(text, start);

    match byte {
        b'#' => {
            // Тег `#:имя`
            if text.get(start + 1) == Some(&b':') {
                return Some(SyntaxSpan {
                    start,
                    end: line_end,
                    kind: SyntaxKind::Tag,
                });
            }
            // Заголовок `#N ` — цифры, затем пробел или конец строки.
            let after = &text[start + 1..line_end];
            let digit_count = after
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digit_count > 0 {
                let level: u32 = after[..digit_count]
                    .iter()
                    .map(|&byte| byte as char)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(1);
                let rest = &after[digit_count..];
                if rest.is_empty() || rest[0] == b' ' {
                    return Some(SyntaxSpan {
                        start,
                        end: line_end,
                        kind: SyntaxKind::Header(level),
                    });
                }
            }
            None
        }
        b'>' => Some(SyntaxSpan {
            start,
            end: line_end,
            kind: SyntaxKind::Quote,
        }),
        b'-' => {
            if len >= 3 && &text[line_start..line_end] == b"---" {
                Some(SyntaxSpan {
                    start,
                    end: start + len,
                    kind: SyntaxKind::ThematicBreak,
                })
            } else if len == 1 && start + 1 < text.len() && text[start + 1] == b' ' {
                Some(SyntaxSpan {
                    start,
                    end: line_end,
                    kind: SyntaxKind::ListItem,
                })
            } else {
                None
            }
        }
        b'*' | b'+' => {
            if len == 1 && start + 1 < text.len() && text[start + 1] == b' ' {
                Some(SyntaxSpan {
                    start,
                    end: line_end,
                    kind: SyntaxKind::ListItem,
                })
            } else {
                None
            }
        }
        b'|' => Some(SyntaxSpan {
            start,
            end: line_end,
            kind: SyntaxKind::TableRow,
        }),
        _ => None,
    }
}

// Нумерованный список `1. ` — цифры от начала строки, затем пробел.
fn try_numbered_list(text: &[u8], start: usize, line_start: usize) -> Option<SyntaxSpan> {
    let line_end = line_end_of(text, start);
    let line = &text[line_start..line_end];
    let rel = start - line_start;
    // Первая не-пробельная позиция строки — начало цифр.
    let digits_start = line
        .iter()
        .position(|&byte| byte != b' ' && byte != b'\t')?;
    // До `.` — только цифры, после — пробел или конец строки.
    if digits_start < rel
        && line[digits_start..rel]
            .iter()
            .all(|byte| byte.is_ascii_digit())
        && line.get(rel + 1).is_none_or(|&byte| byte == b' ')
    {
        Some(SyntaxSpan {
            start: line_start,
            end: line_end,
            kind: SyntaxKind::ListItem,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Прогоняет текст через однопроходный парсер.
    fn parse_spans(text: &str) -> Vec<SyntaxSpan> {
        crate::engine::parser::parse_document(text.as_bytes()).1
    }

    #[test]
    fn bold_closed() {
        // **жирный)) = 2 + 12 (жирный) + 2 = 16 байт
        let spans = parse_spans("**жирный))");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 16,
                kind: SyntaxKind::Bold
            }]
        );
    }

    #[test]
    fn bold_italic_layered_same_text() {
        // **//текст)) = 2 + 2 + 10 (текст) + 2 = 16 байт
        // bold от **, italic от //, одна закрывашка; pop LIFO → italic первым
        let spans = parse_spans("**//текст))");
        assert_eq!(
            spans,
            vec![
                SyntaxSpan {
                    start: 2,
                    end: 16,
                    kind: SyntaxKind::Italic
                },
                SyntaxSpan {
                    start: 0,
                    end: 16,
                    kind: SyntaxKind::Bold
                },
            ]
        );
    }

    #[test]
    fn no_close_discarded() {
        assert!(parse_spans("**жирный").is_empty());
    }

    #[test]
    fn space_before_close_not_a_closer() {
        // перед )) пробел — не закрывашка
        assert!(parse_spans("**жирный ))").is_empty());
    }

    #[test]
    fn space_after_open_not_an_open() {
        assert!(parse_spans("** жирный))").is_empty());
    }

    #[test]
    fn close_without_open_discarded() {
        assert!(parse_spans("просто )) текст").is_empty());
    }

    #[test]
    fn inline_reset_on_newline() {
        // ** на строке 1, )) на строке 2 — inline не выходит за строку
        let spans = parse_spans("**незакрыто\n))");
        assert!(spans.is_empty());
    }

    #[test]
    fn header_span() {
        // #1 Заголовок — заголовок уровня 1, спан до конца строки.
        // "#1 " = 3 байта + "Заголовок" = 9 символов * 2 = 18 → 21 байт
        let spans = parse_spans("#1 Заголовок");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 21,
                kind: SyntaxKind::Header(1)
            }]
        );
    }

    #[test]
    fn header_requires_space() {
        // #1Заголовок — без пробела после цифр, не заголовок
        assert!(parse_spans("#1Заголовок").is_empty());
    }

    #[test]
    fn comment_line() {
        // %%скрыто} — комментарий до }, без пробелов
        // "%%" = 2 + "скрыто" = 6*2 = 12 + "}" = 1 → 15 байт
        let spans = parse_spans("%%скрыто}");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 15,
                kind: SyntaxKind::Comment
            }]
        );
    }

    #[test]
    fn comment_unclosed_discarded() {
        // %% без } — незавершённая конструкция, выбрасывается
        assert!(parse_spans("%%скрыто").is_empty());
    }

    #[test]
    fn comment_mid_line() {
        // line-level маркер работает с любого места строки
        let spans = parse_spans("Текст %%комментарий}");
        assert!(spans.iter().any(|span| span.kind == SyntaxKind::Comment));
    }

    #[test]
    fn comment_space_after_marker_invalid() {
        // пробел после %% — не комментарий (правило «без пробелов»)
        assert!(parse_spans("%% скрыто}").is_empty());
    }

    #[test]
    fn comment_space_before_close_invalid() {
        // пробел перед } — не закрывашка
        assert!(parse_spans("%%скрыто }").is_empty());
    }

    #[test]
    fn spoiler_line() {
        let spans = parse_spans("!!спойлер: текст}");
        assert!(spans.iter().any(|span| span.kind == SyntaxKind::Spoiler));
    }

    #[test]
    fn spoiler_mid_line() {
        let spans = parse_spans("Текст !!скрытое содержимое}");
        assert!(spans.iter().any(|span| span.kind == SyntaxKind::Spoiler));
    }

    #[test]
    fn formula_line() {
        let spans = parse_spans("x = 5 $$sqrt(x)}");
        assert!(spans.iter().any(|span| span.kind == SyntaxKind::Formula));
    }

    #[test]
    fn brace_without_open_discarded() {
        // } без открытого line-level состояния — не закрывашка
        assert!(parse_spans("просто } текст").is_empty());
    }

    #[test]
    fn quote_and_list() {
        let spans = parse_spans("> цитата\n- элемент");
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert_eq!(kinds, vec![SyntaxKind::Quote, SyntaxKind::ListItem]);
    }

    #[test]
    fn tag_at_line_start() {
        // #:важно = 2 + 10 = 12 байт
        let spans = parse_spans("#:важно");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 12,
                kind: SyntaxKind::Tag
            }]
        );
    }

    #[test]
    fn hash_mid_line_not_header() {
        // *100# в тексте — не заголовок и не тег
        let spans = parse_spans("код *100#");
        assert!(
            !spans
                .iter()
                .any(|span| matches!(span.kind, SyntaxKind::Header(_) | SyntaxKind::Tag))
        );
    }

    #[test]
    fn thematic_break() {
        let spans = parse_spans("---");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 3,
                kind: SyntaxKind::ThematicBreak
            }]
        );
    }

    // ─── Полная палитра синтаксиса из docs/syntax.md ─────────────

    #[test]
    fn full_inline_palette() {
        // Каждый inline-маркер из таблицы раздела 1.
        let cases: &[(&str, SyntaxKind)] = &[
            ("**жирный))", SyntaxKind::Bold),
            ("//курсив))", SyntaxKind::Italic),
            ("__подчёркивание))", SyntaxKind::Underline),
            ("~~зачёркивание))", SyntaxKind::Strikethrough),
            ("==подсветка))", SyntaxKind::Highlight),
            ("++вставка))", SyntaxKind::Insertion),
            ("--удаление))", SyntaxKind::Deletion),
            ("''верхний))", SyntaxKind::Superscript),
            (",,нижний))", SyntaxKind::Subscript),
            ("$x+y))", SyntaxKind::Formula),
        ];
        for (text, expected) in cases {
            let spans = parse_spans(text);
            assert_eq!(spans.len(), 1, "текст: {text}");
            assert_eq!(spans[0].kind, *expected, "текст: {text}");
        }
    }

    #[test]
    fn full_line_level_palette() {
        // Line-level маркеры из раздела 2 — закрываются `}`.
        let cases: &[(&str, SyntaxKind)] = &[
            ("%%комментарий}", SyntaxKind::Comment),
            ("$$sqrt(x)}", SyntaxKind::Formula),
            ("!!спойлер}", SyntaxKind::Spoiler),
            ("!!заголовок:текст}", SyntaxKind::Spoiler),
        ];
        for (text, expected) in cases {
            let spans = parse_spans(text);
            assert_eq!(spans.len(), 1, "текст: {text}");
            assert_eq!(spans[0].kind, *expected, "текст: {text}");
        }
    }

    #[test]
    fn full_line_start_palette() {
        // Структурные маркеры из раздела 7.
        let cases: &[(&str, SyntaxKind)] = &[
            ("#1 Заголовок", SyntaxKind::Header(1)),
            ("#:тег", SyntaxKind::Tag),
            ("- элемент", SyntaxKind::ListItem),
            ("1. элемент", SyntaxKind::ListItem),
            ("> текст", SyntaxKind::Quote),
            ("---", SyntaxKind::ThematicBreak),
            ("| cell | cell |", SyntaxKind::TableRow),
        ];
        for (text, expected) in cases {
            let spans = parse_spans(text);
            assert!(
                spans.iter().any(|span| span.kind == *expected),
                "текст: {text}, спаны: {spans:?}"
            );
        }
    }

    #[test]
    fn nested_inline_layers() {
        // **//текст)) — bold и italic на одном тексте (раздел 10).
        let spans = parse_spans("**//текст))");
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert_eq!(kinds, vec![SyntaxKind::Italic, SyntaxKind::Bold]);
    }

    #[test]
    fn real_document_example() {
        // Документ в стиле «Быстрый пример» из docs/syntax.md.
        let text = "#1 Что такое Zoll?\n\n\
            Zoll — это **язык разметки)) с поддержкой //курсива)), \
            __подчёркивания)) и ~~зачёркивания)).\n\n\
            ==Важно:)) этот текст подсвечен.\n\n\
            Текст %%эта часть не видна}\n\n\
            Обычный текст !!а это спойлер до конца строки}\n\n\
            x = 5 $$sqrt(x)}\n";
        let spans = parse_spans(text);
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Header(1)));
        assert!(kinds.contains(&SyntaxKind::Bold));
        assert!(kinds.contains(&SyntaxKind::Italic));
        assert!(kinds.contains(&SyntaxKind::Underline));
        assert!(kinds.contains(&SyntaxKind::Strikethrough));
        assert!(kinds.contains(&SyntaxKind::Highlight));
        assert!(kinds.contains(&SyntaxKind::Comment));
        assert!(kinds.contains(&SyntaxKind::Spoiler));
        assert!(kinds.contains(&SyntaxKind::Formula));
    }
}
