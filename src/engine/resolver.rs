//! Этап 4: Syntax Resolver.
//!
//! Превращает маркеры-последовательности в синтаксические диапазоны
//! (`SyntaxSpan { start, end, kind }`) в байтовых координатах.
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

use crate::engine::collector::Marker;
use crate::engine::line_map::LineMap;

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

// Разрешает маркеры в синтаксические диапазоны.
pub fn resolve(text: &[u8], markers: &[Marker], line_map: &LineMap) -> Vec<SyntaxSpan> {
    let mut spans = Vec::new();
    // Стек открытых inline-маркеров: (свойство, позиция открытия).
    let mut inline_stack: Vec<(SyntaxKind, usize)> = Vec::new();

    for marker in markers {
        match marker.byte {
            b'\n' => {
                // Inline не выходит за пределы строки — сбрасываем.
                inline_stack.clear();
            }
            b')' if marker.len() >= 2 => {
                // Универсальная inline-закрывашка.
                if inline_stack.is_empty() {
                    continue; // нет открытого состояния — не закрывашка
                }
                // Правило пробелов: перед `))` не должно быть пробела.
                if marker.start > 0 && text[marker.start - 1] == b' ' {
                    continue;
                }
                let close_end = marker.end;
                while let Some((kind, open_start)) = inline_stack.pop() {
                    spans.push(SyntaxSpan {
                        start: open_start,
                        end: close_end,
                        kind,
                    });
                }
            }
            _ => {
                // Line-маркеры — только в начале строки.
                if is_line_start(marker.start, line_map, text)
                    && let Some(span) = try_line_marker(text, marker, line_map)
                {
                    spans.push(span);
                    continue;
                }
                // Inline-открытие.
                if let Some(kind) = inline_open(marker.byte, marker.len()) {
                    // Правило пробелов: после открывашки не должно быть пробела.
                    if marker.end < text.len() && text[marker.end] == b' ' {
                        continue;
                    }
                    inline_stack.push((kind, marker.start));
                }
            }
        }
    }

    spans
}

// `true`, если байт `pos` — первая не-пробельная позиция своей строки.
fn is_line_start(pos: usize, line_map: &LineMap, text: &[u8]) -> bool {
    let (line_start, _) = line_map.line_bounds(line_map.line_at(pos));
    text[line_start..pos]
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

// Line-маркер в начале строки.
fn try_line_marker(text: &[u8], marker: &Marker, line_map: &LineMap) -> Option<SyntaxSpan> {
    let (line_start, line_end) = line_map.line_bounds(line_map.line_at(marker.start));
    let line = &text[line_start..line_end.min(text.len())];
    let rel = marker.start - line_start; // позиция маркера внутри строки

    match marker.byte {
        b'#' => {
            // Тег `#:имя`
            if line.get(rel + 1) == Some(&b':') {
                return Some(SyntaxSpan {
                    start: marker.start,
                    end: line_end.min(text.len()),
                    kind: SyntaxKind::Tag,
                });
            }
            // Заголовок `#N#`
            let after = &line[rel + 1..];
            let digits: String = after
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .map(|&byte| byte as char)
                .collect();
            if !digits.is_empty() {
                let level = digits.parse::<u32>().unwrap_or(1);
                let rest = &after[digits.len()..];
                if let Some(close_rel) = rest.iter().position(|&byte| byte == b'#') {
                    let end = marker.start + 1 + digits.len() + close_rel + 1;
                    return Some(SyntaxSpan {
                        start: marker.start,
                        end,
                        kind: SyntaxKind::Header(level),
                    });
                }
            }
            None
        }
        b'%' if marker.len() >= 2 => Some(SyntaxSpan {
            start: marker.start,
            end: line_end.min(text.len()),
            kind: SyntaxKind::Comment,
        }),
        b'!' if marker.len() >= 2 => Some(SyntaxSpan {
            start: marker.start,
            end: line_end.min(text.len()),
            kind: SyntaxKind::Spoiler,
        }),
        b'$' if marker.len() >= 2 => Some(SyntaxSpan {
            start: marker.start,
            end: line_end.min(text.len()),
            kind: SyntaxKind::Formula,
        }),
        b'>' => Some(SyntaxSpan {
            start: marker.start,
            end: line_end.min(text.len()),
            kind: SyntaxKind::Quote,
        }),
        b'-' => {
            if marker.len() >= 3 && line == b"---" {
                Some(SyntaxSpan {
                    start: marker.start,
                    end: marker.end,
                    kind: SyntaxKind::ThematicBreak,
                })
            } else if marker.len() == 1 && marker.end < text.len() && text[marker.end] == b' ' {
                Some(SyntaxSpan {
                    start: marker.start,
                    end: line_end.min(text.len()),
                    kind: SyntaxKind::ListItem,
                })
            } else {
                None
            }
        }
        b'*' | b'+' => {
            if marker.len() == 1 && marker.end < text.len() && text[marker.end] == b' ' {
                Some(SyntaxSpan {
                    start: marker.start,
                    end: line_end.min(text.len()),
                    kind: SyntaxKind::ListItem,
                })
            } else {
                None
            }
        }
        b'|' => Some(SyntaxSpan {
            start: marker.start,
            end: line_end.min(text.len()),
            kind: SyntaxKind::TableRow,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::collector::collect;
    use crate::engine::line_map::LineMap;
    use crate::engine::simd::scan;

    fn resolve_text(text: &str) -> Vec<SyntaxSpan> {
        let events = scan(text.as_bytes(), crate::engine::INTERESTING_BYTES);
        let line_map = LineMap::from_events(&events);
        let markers = collect(&events);
        resolve(text.as_bytes(), &markers, &line_map)
    }

    #[test]
    fn bold_closed() {
        // **жирный)) = 2 + 12 (жирный) + 2 = 16 байт
        let spans = resolve_text("**жирный))");
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
        let spans = resolve_text("**//текст))");
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
        assert!(resolve_text("**жирный").is_empty());
    }

    #[test]
    fn space_before_close_not_a_closer() {
        // перед )) пробел — не закрывашка
        assert!(resolve_text("**жирный ))").is_empty());
    }

    #[test]
    fn space_after_open_not_an_open() {
        assert!(resolve_text("** жирный))").is_empty());
    }

    #[test]
    fn close_without_open_discarded() {
        assert!(resolve_text("просто )) текст").is_empty());
    }

    #[test]
    fn inline_reset_on_newline() {
        // ** на строке 1, )) на строке 2 — inline не выходит за строку
        let spans = resolve_text("**незакрыто\n))");
        assert!(spans.is_empty());
    }

    #[test]
    fn header_span() {
        // спан покрывает маркер #1# (3 байта)
        let spans = resolve_text("#1# Заголовок");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 3,
                kind: SyntaxKind::Header(1)
            }]
        );
    }

    #[test]
    fn comment_line() {
        // %% скрыто = 2 + 1 + 12 = 15 байт
        let spans = resolve_text("%% скрыто");
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
    fn spoiler_line() {
        let spans = resolve_text("!!спойлер: текст");
        assert!(spans.iter().any(|span| span.kind == SyntaxKind::Spoiler));
    }

    #[test]
    fn quote_and_list() {
        let spans = resolve_text("> цитата\n- элемент");
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert_eq!(kinds, vec![SyntaxKind::Quote, SyntaxKind::ListItem]);
    }

    #[test]
    fn tag_at_line_start() {
        // #:важно = 2 + 10 = 12 байт
        let spans = resolve_text("#:важно");
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
        let spans = resolve_text("код *100#");
        assert!(
            !spans
                .iter()
                .any(|span| matches!(span.kind, SyntaxKind::Header(_) | SyntaxKind::Tag))
        );
    }

    #[test]
    fn thematic_break() {
        let spans = resolve_text("---");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 3,
                kind: SyntaxKind::ThematicBreak
            }]
        );
    }
}
