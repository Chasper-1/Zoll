//! Классификация маркеров: превращает маркеры-последовательности в
//! синтаксические диапазоны (`SyntaxSpan { start, end, kind }`) в байтовых
//! координатах.
//!
//! Два отдельных этапа:
//!
//! 1. `parser::parse_document` делает из регистров SIMD готовые строки
//!    (карта `\n`).
//! 2. `process_marker` разбирает каждый маркер, и границы его строки
//!    (`line_start`, `line_end`) уже готовы из карты.
//!
//! Никакого сканирования текста: всё, что нужно, уже найдено SIMD.
//!
//! ## Уровни маркеров (docs/syntax.md, раздел 9)
//!
//! Тип маркера определяется самим маркером (символом и их количеством),
//! а не позицией в строке:
//!
//! - **Inline** (`**`, `//`, `%`, `$`, `!`, ...) — в любом месте строки,
//!   закрываются `))`. Стек открытых маркеров; `))` закрывает все разом.
//! - **Line (close)** (`%%`, `$$`, `!!`, `>`) — только в начале строки,
//!   закрываются `}` (приоритетнее) или автоматически до конца строки.
//! - **Line (not-close)** (`#1`, `#:`, `-`, `1.`, `---`, `|`) — только
//!   в начале строки, диапазон сразу до конца строки.
//! - **Block** (`%%%`, `$$$`, `!!!`, `@@`) — только в начале строки,
//!   переживают строки, закрываются `}` в начале строки.
//!
//! Правила пробелов: после открывающего маркера (кроме цитаты `>`) и перед
//! закрывающим пробел запрещён.
//!
//! Спаны создаются только в финальном виде: line-close держится в одном
//! слоте на строку и рождается в момент `}` или в конце строки. Благодаря
//! этому каждый спан можно сразу отдавать синку (стрим) — править его
//! потом не нужно.

use crate::engine::api::{SpanSink, dispatch_span};
use crate::engine::parser::{CAT_DIGIT, CATEGORY};

// Таблицы inline-маркеров: индекс по байту, значение — вид конструкции.
// Вместо match-веток в горячем цикле: одна загрузка из L1. Две таблицы —
// по длине маркера (2 и 1), потому что один байт может быть и line-,
// и inline-маркером (`*` — список и bold, `$` — формула и line/block).
const INLINE2: [Option<SyntaxKind>; 256] = {
    let mut table = [None; 256];
    table[b'*' as usize] = Some(SyntaxKind::Bold);
    table[b'/' as usize] = Some(SyntaxKind::Italic);
    table[b'_' as usize] = Some(SyntaxKind::Underline);
    table[b'~' as usize] = Some(SyntaxKind::Strikethrough);
    table[b'=' as usize] = Some(SyntaxKind::Highlight);
    table[b'+' as usize] = Some(SyntaxKind::Insertion);
    table[b'-' as usize] = Some(SyntaxKind::Deletion);
    table[b'\'' as usize] = Some(SyntaxKind::Superscript);
    table[b',' as usize] = Some(SyntaxKind::Subscript);
    table
};

const INLINE1: [Option<SyntaxKind>; 256] = {
    let mut table = [None; 256];
    table[b'$' as usize] = Some(SyntaxKind::FormulaInline);
    table[b'%' as usize] = Some(SyntaxKind::CommentInline);
    table[b'!' as usize] = Some(SyntaxKind::SpoilerInline);
    table[b'`' as usize] = Some(SyntaxKind::CodeInline);
    table
};

// Вид синтаксической конструкции.
//
// Каждый вид — отдельная конструкция: inline/line/block варианты одного
// семейства разведены (например, `%`/`%%`/`%%%` — CommentInline,
// CommentLine, CommentBlock), чтобы редактор получал их по отдельным
// ручкам и не определял уровень сам.
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
    FormulaInline,
    CommentInline,
    SpoilerInline,
    CodeInline,
    // Line
    Header(u8),
    Tag,
    Quote,
    ListItem,
    TableRow,
    ThematicBreak,
    FormulaLine,
    CommentLine,
    SpoilerLine,
    CodeLine,
    // Block
    FormulaBlock,
    CommentBlock,
    SpoilerBlock,
    CodeBlock,
    Metadata,
}

// Синтаксический диапазон в байтовых координатах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start: usize,
    pub end: usize,
    pub kind: SyntaxKind,
}

// Состояние разбора: текст, текущая строка, стеки и диапазоны.
pub(crate) struct ResolveState<'a, 's, S: SpanSink + ?Sized> {
    pub text: &'a [u8],
    // Границы текущей строки — известны из карты строк (этап 1).
    pub line_start: usize,
    pub line_end: usize,
    pub spans: Vec<SyntaxSpan>,
    // Открытые inline-маркеры (**, //, %, $, !, ...): не выходят за строку.
    pub inline_stack: Vec<(SyntaxKind, usize)>,
    // Блочные конструкции (%%%/$$$/!!!/@@): переживают строки, закрываются
    // `}` строго в начале строки.
    pub block_stack: Vec<(SyntaxKind, usize)>,
    // Открытый line-close маркер текущей строки (%%/$$/!!/`>`): один на
    // строку, потому что открывается только в начале строки. Спан рождается
    // в момент готовности — при `}` или в конце строки.
    pub pending_line_close: Option<(SyntaxKind, usize)>,
    // Стрим-синк: если есть, каждый спан отдаётся сразу при создании.
    // Generic: мономорфизация убирает виртуальный вызов из горячего пути.
    pub sink: Option<&'s mut S>,
}

impl<'a, 's, S: SpanSink + ?Sized> ResolveState<'a, 's, S> {
    pub(crate) fn new(text: &'a [u8]) -> Self {
        // Ёмкость сразу по размеру текста: 1/16 документа. Список спанов
        // растёт по ходу парсинга, и без запаса перевыделение копировало бы
        // всё накопленное (~13 раз за большой документ). С запасом 1/16
        // реаллоки исчезают: чуть больше памяти сразу — но это дёшево.
        ResolveState {
            text,
            line_start: 0,
            line_end: text.len(),
            spans: Vec::with_capacity(text.len() / 12 + 16),
            inline_stack: Vec::with_capacity(8),
            block_stack: Vec::with_capacity(4),
            pending_line_close: None,
            sink: None,
        }
    }

    // Создаёт спан и сразу отдаёт его синку, если он есть.
    // Спан всегда финальный: править его после создания не нужно.
    #[inline]
    pub(crate) fn emit(&mut self, span: SyntaxSpan) {
        self.spans.push(span);
        if let Some(sink) = self.sink.as_deref_mut() {
            dispatch_span(sink, span);
        }
    }
}

// Разбирает завершённый маркер `byte` длины `len`, начинающийся в `start`.
//
// Границы текущей строки (`line_start`, `line_end`) уже готовы из карты
// строк — сканировать текст не нужно. Может не открыть/закрыть ничего:
// маркер просто игнорируется. Тонкий диспетчер: каждая ветка — отдельная
// функция (в release инлайнится, оверхеда нет).
//
// inline(always): вызывается из маркерного цикла ~10-15 тысяч раз за
// парсинг. Обычный `#[inline]` — лишь подсказка: при codegen-units=16
// компилятор может вынести функцию в другой юнит и не заинлайнить,
// что стоит ~5 мкс на вызовах. Принудительный инлайн делает
// производительность независимой от разбиения на юниты.
#[inline(always)]
pub(crate) fn process_marker<S: SpanSink + ?Sized>(
    state: &mut ResolveState<'_, '_, S>,
    byte: u8,
    start: usize,
    len: usize,
) {
    let end = start + len;
    match byte {
        b')' if len >= 2 => close_inline_markers(state, start, end),
        b'}' => close_brace(state, start, end),
        b';' => try_semicolon_list(state, start),
        _ => open_marker(state, byte, len, start, end),
    }
}

// `}` — одна скобка для двух уровней, различие по позиции:
// - в начале строки → закрытие блока (%%%/$$$/!!!/@@)
// - mid-line → закрытие line-close (%%/$$/!!/`>`)
#[inline]
fn close_brace<S: SpanSink + ?Sized>(
    state: &mut ResolveState<'_, '_, S>,
    start: usize,
    end: usize,
) {
    if start == state.line_start {
        // Блок: закрывается строго в начале строки, без правила пробелов.
        if let Some((kind, open_position)) = state.block_stack.pop() {
            state.emit(SyntaxSpan {
                start: open_position,
                end,
                kind,
            });
        }
    } else {
        // Line-close: спан рождается здесь, уже финальный.
        let text = state.text;
        // Правило пробелов: перед `}` не должно быть пробела.
        if start > 0 && text[start - 1] == b' ' {
            return;
        }
        if let Some((kind, open_position)) = state.pending_line_close.take() {
            state.emit(SyntaxSpan {
                start: open_position,
                end,
                kind,
            });
        }
    }
}

// Универсальная inline-закрывашка `))`: закрывает все открытые inline.
#[inline]
fn close_inline_markers<S: SpanSink + ?Sized>(
    state: &mut ResolveState<'_, '_, S>,
    start: usize,
    end: usize,
) {
    let text = state.text;
    // Нет открытого состояния — не закрывашка.
    if state.inline_stack.is_empty() {
        return;
    }
    // Правило пробелов: перед `))` не должно быть пробела.
    if start > 0 && text[start - 1] == b' ' {
        return;
    }
    while let Some((kind, open_position)) = state.inline_stack.pop() {
        state.emit(SyntaxSpan {
            start: open_position,
            end,
            kind,
        });
    }
}

// Нумерованный список `1; ` — цифры от начала строки, затем пробел.
// Проверка цифр — через таблицу категорий (CATEGORY), без is_ascii_digit.
#[inline]
fn try_semicolon_list<S: SpanSink + ?Sized>(state: &mut ResolveState<'_, '_, S>, start: usize) {
    let text = state.text;
    let mut numbered = start > state.line_start;
    if numbered {
        for &byte in &text[state.line_start..start] {
            if CATEGORY[byte as usize] != CAT_DIGIT {
                numbered = false;
                break;
            }
        }
    }
    if numbered && text.get(start + 1).is_none_or(|&byte| byte == b' ') {
        state.emit(SyntaxSpan {
            start: state.line_start,
            end: state.line_end,
            kind: SyntaxKind::ListItem,
        });
    }
}

// Открытие маркера: блок, line-close, line (not-close) или inline.
// Единый match (byte, len): один диспетчер вместо цепочки из пяти
// (block_marker_kind → line_close_marker_kind → is_line_not_close_byte →
// try_line_not_close → inline_marker_kind). Проверка «в начале строки»
// выполняется внутри веток — только для line-маркеров, inline не платит.
// inline(always): вызывается из маркерного цикла ~10-15 тысяч раз за
// парсинг; принудительный инлайн делает производительность независимой
// от разбиения на codegen-юниты.
#[inline(always)]
fn open_marker<S: SpanSink + ?Sized>(
    state: &mut ResolveState<'_, '_, S>,
    byte: u8,
    len: usize,
    start: usize,
    end: usize,
) {
    let text = state.text;
    match (byte, len) {
        // Блок (%%%/$$$/!!!/@@) — строго в начале строки.
        (b'%', 3) | (b'$', 3) | (b'!', 3) | (b'`', 3) | (b'@', 2) => {
            if start == state.line_start {
                let kind = match byte {
                    b'%' => SyntaxKind::CommentBlock,
                    b'$' => SyntaxKind::FormulaBlock,
                    b'!' => SyntaxKind::SpoilerBlock,
                    b'`' => SyntaxKind::CodeBlock,
                    _ => SyntaxKind::Metadata,
                };
                state.block_stack.push((kind, start));
            }
        }
        // Line-close открытие (%%/$$/!!/`>`) — только в начале строки.
        // Спан не создаётся: маркер ждёт в слоте до `}` или конца строки.
        (b'%', 2) | (b'$', 2) | (b'!', 2) | (b'`', 2) => {
            if start == state.line_start {
                if end < text.len() && text[end] == b' ' {
                    return;
                }
                let kind = match byte {
                    b'%' => SyntaxKind::CommentLine,
                    b'$' => SyntaxKind::FormulaLine,
                    b'!' => SyntaxKind::SpoilerLine,
                    _ => SyntaxKind::CodeLine,
                };
                state.pending_line_close = Some((kind, start));
            }
        }
        (b'>', 1) => {
            if start == state.line_start {
                state.pending_line_close = Some((SyntaxKind::Quote, start));
            }
        }
        // Line (not-close) маркеры — только в начале строки.
        (b'#', _) => {
            if start == state.line_start {
                // Тег `#:имя`
                if text.get(start + 1) == Some(&b':') {
                    state.emit(SyntaxSpan {
                        start,
                        end: state.line_end,
                        kind: SyntaxKind::Tag,
                    });
                    return;
                }
                // Заголовок `#N ` — цифры (таблица CATEGORY), затем пробел
                // или конец строки. Уровень u8: максимум 3 цифры, ≤ 255.
                let after = &text[start + 1..state.line_end];
                let mut level: u32 = 0;
                let mut digits = 0;
                for &byte in after
                    .iter()
                    .take_while(|&&b| CATEGORY[b as usize] == CAT_DIGIT)
                    .take(3)
                {
                    level = level * 10 + (byte - b'0') as u32;
                    digits += 1;
                }
                if digits > 0 {
                    let rest = &after[digits..];
                    if (rest.is_empty() || rest[0] == b' ') && level <= 255 {
                        state.emit(SyntaxSpan {
                            start,
                            end: state.line_end,
                            kind: SyntaxKind::Header(level as u8),
                        });
                    }
                }
            }
        }
        (b'-', 1) => {
            if start == state.line_start && start + 1 < text.len() && text[start + 1] == b' ' {
                state.emit(SyntaxSpan {
                    start,
                    end: state.line_end,
                    kind: SyntaxKind::ListItem,
                });
            }
        }
        (b'-', _) if len >= 3 => {
            if start == state.line_start && &text[start..state.line_end] == b"---" {
                state.emit(SyntaxSpan {
                    start,
                    end: state.line_end,
                    kind: SyntaxKind::ThematicBreak,
                });
            }
        }
        (b'*', 1) | (b'+', 1) => {
            if start == state.line_start && start + 1 < text.len() && text[start + 1] == b' ' {
                state.emit(SyntaxSpan {
                    start,
                    end: state.line_end,
                    kind: SyntaxKind::ListItem,
                });
            }
        }
        (b'|', _) => {
            if start == state.line_start {
                state.emit(SyntaxSpan {
                    start,
                    end: state.line_end,
                    kind: SyntaxKind::TableRow,
                });
            }
        }
        // Inline-открытие: таблица по байту (одна загрузка из L1 вместо
        // match-веток), правило пробелов + push в стек.
        (b'*', 2)
        | (b'/', 2)
        | (b'_', 2)
        | (b'~', 2)
        | (b'=', 2)
        | (b'+', 2)
        | (b'-', 2)
        | (b'\'', 2)
        | (b',', 2) => {
            if let Some(kind) = INLINE2[byte as usize] {
                push_inline(state, kind, start, end);
            }
        }
        (b'$', 1) | (b'%', 1) | (b'!', 1) | (b'`', 1) => {
            if let Some(kind) = INLINE1[byte as usize] {
                push_inline(state, kind, start, end);
            }
        }
        _ => {}
    }
}

// Inline-открытие: правило пробелов (после открывашки не должно быть
// пробела) + push в стек.
#[inline(always)]
fn push_inline<S: SpanSink + ?Sized>(
    state: &mut ResolveState<'_, '_, S>,
    kind: SyntaxKind,
    start: usize,
    end: usize,
) {
    if end < state.text.len() && state.text[end] == b' ' {
        return;
    }
    state.inline_stack.push((kind, start));
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
    fn code_inline_closed() {
        // `code)) = 1 + 4 (code) + 2 = 7 байт
        let spans = parse_spans("`code))");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 7,
                kind: SyntaxKind::CodeInline
            }]
        );
    }

    #[test]
    fn code_inline_mid_line() {
        // a `code)) = 2 + 1 + 4 + 2 = 9 байт
        let spans = parse_spans("a `code))");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 2,
                end: 9,
                kind: SyntaxKind::CodeInline
            }]
        );
    }

    #[test]
    fn code_inline_unclosed_discarded() {
        assert!(parse_spans("`code").is_empty());
    }

    #[test]
    fn code_inline_space_after_open_invalid() {
        assert!(parse_spans("` code))").is_empty());
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
                kind: SyntaxKind::CommentLine
            }]
        );
    }

    #[test]
    fn comment_unclosed_to_end_of_line() {
        // %% без } — line-close действует до конца строки
        let spans = parse_spans("%%скрыто");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CommentLine);
    }

    #[test]
    fn comment_mid_line_ignored() {
        // line-close маркер работает только с начала строки — mid-line %% текст.
        assert!(parse_spans("Текст %%комментарий}").is_empty());
    }

    #[test]
    fn comment_space_after_marker_invalid() {
        // пробел после %% — не комментарий (правило «без пробелов»)
        assert!(parse_spans("%% скрыто}").is_empty());
    }

    #[test]
    fn code_line() {
        // ``код} — строка кода до }, без пробелов
        // "``" = 2 + "код" = 6 + "}" = 1 → 9 байт
        let spans = parse_spans("``код}");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 9,
                kind: SyntaxKind::CodeLine
            }]
        );
    }

    #[test]
    fn code_line_unclosed_to_end_of_line() {
        // `` без } — line-close действует до конца строки
        let spans = parse_spans("``код");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CodeLine);
    }

    #[test]
    fn code_line_space_after_marker_invalid() {
        // пробел после `` — не код (правило «без пробелов»)
        assert!(parse_spans("`` код}").is_empty());
    }

    #[test]
    fn comment_space_before_close_goes_to_eol() {
        // пробел перед } — не закрывашка, комментарий идёт до конца строки
        let spans = parse_spans("%%скрыто }");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CommentLine);
    }

    #[test]
    fn spoiler_line() {
        let spans = parse_spans("!!спойлер: текст}");
        assert!(
            spans
                .iter()
                .any(|span| span.kind == SyntaxKind::SpoilerLine)
        );
    }

    #[test]
    fn spoiler_mid_line_ignored() {
        // line-close маркер работает только с начала строки — mid-line !! текст.
        assert!(parse_spans("Текст !!скрытое содержимое}").is_empty());
    }

    #[test]
    fn formula_line() {
        let spans = parse_spans("$$sqrt(x)}");
        assert!(
            spans
                .iter()
                .any(|span| span.kind == SyntaxKind::FormulaLine)
        );
    }

    #[test]
    fn brace_without_open_discarded() {
        // } без открытого line-close состояния — не закрывашка
        assert!(parse_spans("просто } текст").is_empty());
    }

    #[test]
    fn line_close_lifo_on_same_line() {
        // %%a $$b} x — $$ mid-line не маркер (только начало строки),
        // `}` закрывает единственный line-close строки (%%).
        let spans = parse_spans("%%a $$b} x");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 8,
                kind: SyntaxKind::CommentLine
            }]
        );
    }

    #[test]
    fn brace_mid_line_does_not_close_previous_line() {
        // } в середине строки 2 не трогает line-close строки 1.
        let spans = parse_spans("%%a\nb }");
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 3));
    }

    #[test]
    fn quote_and_list() {
        let spans = parse_spans("> цитата\n- элемент");
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert_eq!(kinds, vec![SyntaxKind::Quote, SyntaxKind::ListItem]);
    }

    #[test]
    fn quote_closed_with_brace() {
        // >цитата} — цитата закрывается `}`: 1 + 12 + 1 = 14 байт.
        let spans = parse_spans(">цитата}");
        assert_eq!(
            spans,
            vec![SyntaxSpan {
                start: 0,
                end: 14,
                kind: SyntaxKind::Quote
            }]
        );
    }

    #[test]
    fn quote_unclosed_to_end_of_line() {
        // > без } — цитата действует до конца строки: 1 + 12 = 13 байт.
        let spans = parse_spans(">цитата\nдальше");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Quote);
        assert_eq!((spans[0].start, spans[0].end), (0, 13));
    }

    #[test]
    fn quote_mid_line_ignored() {
        // > не в начале строки — не цитата.
        assert!(parse_spans("текст > цитата").is_empty());
    }

    #[test]
    fn quote_with_space() {
        // > text — пробел после маркера допустим: 1 + 1 + 4 = 6 байт.
        let spans = parse_spans("> text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Quote);
        assert_eq!((spans[0].start, spans[0].end), (0, 6));
    }

    #[test]
    fn quote_space_before_close_goes_to_eol() {
        // пробел перед } — не закрывашка, цитата идёт до конца строки.
        let spans = parse_spans(">цитата }");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Quote);
        assert_eq!((spans[0].start, spans[0].end), (0, 15));
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
            ("$x+y))", SyntaxKind::FormulaInline),
            ("%комментарий))", SyntaxKind::CommentInline),
            ("!скрыто))", SyntaxKind::SpoilerInline),
            ("`код))", SyntaxKind::CodeInline),
        ];
        for (text, expected) in cases {
            let spans = parse_spans(text);
            assert_eq!(spans.len(), 1, "текст: {text}");
            assert_eq!(spans[0].kind, *expected, "текст: {text}");
        }
    }

    #[test]
    fn full_line_level_palette() {
        // Line-close маркеры из раздела 2 — закрываются `}`.
        let cases: &[(&str, SyntaxKind)] = &[
            ("%%комментарий}", SyntaxKind::CommentLine),
            ("$$sqrt(x)}", SyntaxKind::FormulaLine),
            ("!!спойлер}", SyntaxKind::SpoilerLine),
            ("!!заголовок:текст}", SyntaxKind::SpoilerLine),
            ("``код}", SyntaxKind::CodeLine),
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
            ("1; элемент", SyntaxKind::ListItem),
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
    fn header_level_bounds() {
        // Уровень заголовка — u8: максимум 3 цифры, ≤ 255.
        let cases: &[(&str, u8)] = &[
            ("#1 Заголовок", 1),
            ("#12 Заголовок", 12),
            ("#255 Заголовок", 255),
        ];
        for (text, expected) in cases {
            let spans = parse_spans(text);
            assert_eq!(spans.len(), 1, "текст: {text}");
            assert_eq!(
                spans[0].kind,
                SyntaxKind::Header(*expected),
                "текст: {text}"
            );
        }
    }

    #[test]
    fn header_level_out_of_bounds_not_header() {
        // Больше 3 цифр или > 255 — не заголовок.
        let cases = [
            "#256 Заголовок",
            "#1234 Заголовок",
            "#abc Заголовок",
            "#1a Заголовок",
        ];
        for text in cases {
            let spans = parse_spans(text);
            assert!(
                spans.iter().all(|span| span.kind != SyntaxKind::Header(0)),
                "текст: {text}"
            );
        }
    }

    #[test]
    fn semicolon_list_any_number() {
        // `N; ` — любое число цифр от начала строки.
        let cases = ["1; элемент", "42; элемент", "123456789; элемент"];
        for text in cases {
            let spans = parse_spans(text);
            assert!(
                spans.iter().any(|span| span.kind == SyntaxKind::ListItem),
                "текст: {text}"
            );
        }
    }

    #[test]
    fn semicolon_not_list() {
        // Без пробела после `;` — не список.
        let cases = ["1;элемент", "a; элемент", "; элемент"];
        for text in cases {
            let spans = parse_spans(text);
            assert!(
                spans.iter().all(|span| span.kind != SyntaxKind::ListItem),
                "текст: {text}"
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
            %%эта часть не видна}\n\n\
            !!а это спойлер до конца строки}\n\n\
            $$sqrt(x)}\n";
        let spans = parse_spans(text);
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Header(1)));
        assert!(kinds.contains(&SyntaxKind::Bold));
        assert!(kinds.contains(&SyntaxKind::Italic));
        assert!(kinds.contains(&SyntaxKind::Underline));
        assert!(kinds.contains(&SyntaxKind::Strikethrough));
        assert!(kinds.contains(&SyntaxKind::Highlight));
        assert!(kinds.contains(&SyntaxKind::CommentLine));
        assert!(kinds.contains(&SyntaxKind::SpoilerLine));
        assert!(kinds.contains(&SyntaxKind::FormulaLine));
    }

    // ─── Блочные маркеры (%%%/$$$/!!!) ─────────────────────────

    #[test]
    fn block_comment_multiline() {
        // %%% открывает блок, `}` в начале строки закрывает.
        let spans = parse_spans("%%%\nскрыто\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CommentBlock);
    }

    #[test]
    fn block_formula_multiline() {
        let spans = parse_spans("$$$\nx^2\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::FormulaBlock);
    }

    #[test]
    fn block_spoiler_with_title() {
        // Заголовок внутри спана — редактор классифицирует диапазоном.
        let spans = parse_spans("!!!спойлер:\nскрыто\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::SpoilerBlock);
    }

    #[test]
    fn block_unclosed_discarded() {
        // %%% без закрывающего `}` в начале строки — выбрасывается.
        assert!(parse_spans("%%%\nскрыто").is_empty());
    }

    #[test]
    fn block_close_without_open_discarded() {
        // `}` в начале строки без открытого блока — не закрывашка.
        assert!(parse_spans("текст\n}").is_empty());
    }

    #[test]
    fn block_open_mid_line_ignored() {
        // %%% не в начале строки — не блок.
        assert!(parse_spans("текст %%%\nскрыто\n}").is_empty());
    }

    #[test]
    fn block_open_indented_ignored() {
        // Отступы запрещены: %%% не первый символ строки — не блок.
        assert!(parse_spans(" %%%\nскрыто\n}").is_empty());
    }

    #[test]
    fn block_close_indented_ignored() {
        // `}` с отступом — не закрытие блока (блок остаётся открытым).
        assert!(parse_spans("%%%\nскрыто\n }").is_empty());
    }

    #[test]
    fn code_block_multiline() {
        // ``` открывает блок, `}` в начале строки закрывает.
        let spans = parse_spans("```\nкод\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CodeBlock);
    }

    #[test]
    fn code_block_unclosed_discarded() {
        // ``` без закрывающего `}` в начале строки — выбрасывается.
        assert!(parse_spans("```\nкод").is_empty());
    }

    #[test]
    fn code_block_open_mid_line_ignored() {
        // ``` не в начале строки — не блок.
        assert!(parse_spans("текст ```\nкод\n}").is_empty());
    }

    #[test]
    fn code_block_open_indented_ignored() {
        // Отступы запрещены: ``` не первый символ строки — не блок.
        assert!(parse_spans(" ```\nкод\n}").is_empty());
    }

    #[test]
    fn block_and_line_level_coexist() {
        // Блок открыт, внутри строки line-close комментарий закрывается
        // своей `}` mid-line, блок — своей в начале строки.
        let spans = parse_spans("%%%\n%%скрыто}\n}");
        let kinds: Vec<SyntaxKind> = spans.iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![SyntaxKind::CommentLine, SyntaxKind::CommentBlock]
        );
    }

    #[test]
    fn block_survives_multiple_lines() {
        // Содержимое блока на нескольких строках — спан от %%% до `}`.
        let spans = parse_spans("%%%\nстрока 1\nстрока 2\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::CommentBlock);
    }

    // ─── Inline-комментарий % и inline-спойлер ! ────────────────

    #[test]
    fn percent_inline_comment() {
        // % — inline-комментарий: работает в любом месте строки.
        let spans = parse_spans("Текст %скрыто))");
        assert!(
            spans
                .iter()
                .any(|span| span.kind == SyntaxKind::CommentInline)
        );
    }

    #[test]
    fn exclamation_inline_spoiler() {
        // ! — inline-спойлер.
        let spans = parse_spans("Текст !скрыто))");
        assert!(
            spans
                .iter()
                .any(|span| span.kind == SyntaxKind::SpoilerInline)
        );
    }

    #[test]
    fn stray_percent_harmless() {
        // Одиночный % без закрытия — не конструкция.
        assert!(parse_spans("50% текст").is_empty());
    }

    #[test]
    fn stray_exclamation_harmless() {
        // Одиночный ! без закрытия — не конструкция.
        assert!(parse_spans("Привет!").is_empty());
    }

    #[test]
    fn line_close_mid_line_unclosed_to_eol_ignored() {
        // %% в середине строки без } — не маркер (только начало строки).
        assert!(parse_spans("Текст %%скрыто").is_empty());
    }

    // ─── Метаданные (@@) ─────────────────────────────────────────

    #[test]
    fn metadata_block() {
        // @@ открывает блок метаданных, `}` в начале строки закрывает.
        let spans = parse_spans("@@\ntitle: Документ\n}");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, SyntaxKind::Metadata);
    }

    #[test]
    fn metadata_unclosed_discarded() {
        // @@ без закрывающего `}` — выбрасывается.
        assert!(parse_spans("@@\ntitle: Документ").is_empty());
    }

    #[test]
    fn metadata_indented_ignored() {
        // Отступы запрещены: @@ не первый символ строки — не блок.
        assert!(parse_spans(" @@\ntitle: Документ\n}").is_empty());
    }

    #[test]
    fn metadata_mid_line_ignored() {
        // @@ не в начале строки — не блок.
        assert!(parse_spans("текст @@\ntitle: Документ\n}").is_empty());
    }

    #[test]
    fn single_at_not_marker() {
        // Одиночный @ и тройной @@@ — не маркеры.
        assert!(parse_spans("почта user@example.com").is_empty());
        assert!(parse_spans("@@@\nтекст\n}").is_empty());
    }
}
