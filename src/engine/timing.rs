//! Диагностика производительности: внутренние операции по каждому приколу.
//!
//! Время парсинга мгновенное — оно ничего не говорит. Важно, СКОЛЬКО
//! внутренних операций делает каждая конструкция: сравнений, чтений байтов,
//! push/pop стеков, аллокаций спанов.
//!
//! Подсчёт идёт в ОТДЕЛЬНОМ проходе `timing::analyze`, а не в парсере.
//! `Engine::parse` всегда использует чистый путь (`COUNT = false`) — время
//! парсинга НЕ меняется никогда, даже когда фича `timing` включена.

use crate::engine::SyntaxKind;
use std::cell::Cell;

// Ключ тайминга: механизм или конструкция.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingKey {
    // Механизмы
    InlineClose,  // )) — закрытие inline
    LineClose,    // } — закрытие line-level
    NumberedList, // . — нумерованный список
    Ignored,      // маркер, который ничего не сделал
    // Конструкции
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
    Header,
    Comment,
    Spoiler,
    Quote,
    ListItem,
    TableRow,
    Tag,
    ThematicBreak,
}

// Все ключи в порядке объявления — индекс варианта == позиция в массиве.
#[cfg(feature = "timing")]
const ALL_KEYS: [TimingKey; TimingKey::COUNT] = [
    TimingKey::InlineClose,
    TimingKey::LineClose,
    TimingKey::NumberedList,
    TimingKey::Ignored,
    TimingKey::Bold,
    TimingKey::Italic,
    TimingKey::Underline,
    TimingKey::Strikethrough,
    TimingKey::Highlight,
    TimingKey::Insertion,
    TimingKey::Deletion,
    TimingKey::Superscript,
    TimingKey::Subscript,
    TimingKey::Formula,
    TimingKey::Header,
    TimingKey::Comment,
    TimingKey::Spoiler,
    TimingKey::Quote,
    TimingKey::ListItem,
    TimingKey::TableRow,
    TimingKey::Tag,
    TimingKey::ThematicBreak,
];

impl TimingKey {
    pub const COUNT: usize = 22;

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            TimingKey::InlineClose => "inline-закрытие ))",
            TimingKey::LineClose => "line-закрытие }",
            TimingKey::NumberedList => "нумерованный список .",
            TimingKey::Ignored => "игнор (мёртвый маркер)",
            TimingKey::Bold => "bold **",
            TimingKey::Italic => "italic //",
            TimingKey::Underline => "underline __",
            TimingKey::Strikethrough => "strikethrough ~~",
            TimingKey::Highlight => "highlight ==",
            TimingKey::Insertion => "insertion ++",
            TimingKey::Deletion => "deletion --",
            TimingKey::Superscript => "superscript ''",
            TimingKey::Subscript => "subscript ,,",
            TimingKey::Formula => "formula $",
            TimingKey::Header => "заголовок #N",
            TimingKey::Comment => "комментарий %%",
            TimingKey::Spoiler => "спойлер !!",
            TimingKey::Quote => "цитата >",
            TimingKey::ListItem => "список -/*/+",
            TimingKey::TableRow => "таблица |",
            TimingKey::Tag => "тег #:",
            TimingKey::ThematicBreak => "разделитель ---",
        }
    }

    pub fn from_kind(kind: SyntaxKind) -> TimingKey {
        match kind {
            SyntaxKind::Bold => TimingKey::Bold,
            SyntaxKind::Italic => TimingKey::Italic,
            SyntaxKind::Underline => TimingKey::Underline,
            SyntaxKind::Strikethrough => TimingKey::Strikethrough,
            SyntaxKind::Highlight => TimingKey::Highlight,
            SyntaxKind::Insertion => TimingKey::Insertion,
            SyntaxKind::Deletion => TimingKey::Deletion,
            SyntaxKind::Superscript => TimingKey::Superscript,
            SyntaxKind::Subscript => TimingKey::Subscript,
            SyntaxKind::Formula => TimingKey::Formula,
            SyntaxKind::Header(_) => TimingKey::Header,
            SyntaxKind::Comment => TimingKey::Comment,
            SyntaxKind::Spoiler => TimingKey::Spoiler,
            SyntaxKind::Quote => TimingKey::Quote,
            SyntaxKind::ListItem => TimingKey::ListItem,
            SyntaxKind::TableRow => TimingKey::TableRow,
            SyntaxKind::Tag => TimingKey::Tag,
            SyntaxKind::ThematicBreak => TimingKey::ThematicBreak,
        }
    }
}

// Накопитель: вызовы и операции на каждый ключ.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub calls: [u64; TimingKey::COUNT],
    pub ops: [u64; TimingKey::COUNT],
}

impl Timing {
    pub const fn new() -> Self {
        Timing {
            calls: [0; TimingKey::COUNT],
            ops: [0; TimingKey::COUNT],
        }
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static ACCUM: Cell<Timing> = const { Cell::new(Timing::new()) };
    // Текущий маркер в потоке: ключ и накопленные операции.
    // Используется только при COUNT=true (диагностический проход).
    static CURRENT: Cell<(TimingKey, u64)> = const { Cell::new((TimingKey::Ignored, 0)) };
}

// Счётчик операций одного маркера.
//
// `COUNT = false` — нулевой размер, все методы пустые (const-свёртка),
// компилятор выкидывает таймер целиком: время парсинга не меняется.
// `COUNT = true` — операции копятся в потоковом слоте CURRENT. Ключ
// назначается внутри ветки через `set`, операции через `op`/`op_n`.
// При выходе из функции (включая ранние return) всё пишется в свой ключ.
pub struct MarkerTimer<const COUNT: bool>;

pub fn begin<const COUNT: bool>() -> MarkerTimer<COUNT> {
    if COUNT {
        CURRENT.set((TimingKey::Ignored, 0));
    }
    MarkerTimer
}

impl<const COUNT: bool> MarkerTimer<COUNT> {
    pub fn set(&self, key: TimingKey) {
        if COUNT {
            let (_, ops) = CURRENT.get();
            CURRENT.set((key, ops));
        }
    }

    pub fn op(&self) {
        if COUNT {
            let (key, ops) = CURRENT.get();
            CURRENT.set((key, ops + 1));
        }
    }

    pub fn op_n(&self, n: u64) {
        if COUNT {
            let (key, ops) = CURRENT.get();
            CURRENT.set((key, ops + n));
        }
    }
}

impl<const COUNT: bool> Drop for MarkerTimer<COUNT> {
    fn drop(&mut self) {
        if COUNT {
            let (key, ops) = CURRENT.get();
            let idx = key.index();
            ACCUM.with(|acc| {
                let mut t = acc.get();
                t.calls[idx] += 1;
                t.ops[idx] += ops;
                acc.set(t);
            });
        }
    }
}

// Отдельный проход с подсчётом операций. Парсер (Engine::parse) не трогает:
// он всегда идёт по чистому пути `COUNT = false`.
#[cfg(feature = "timing")]
pub fn analyze(text: &[u8]) {
    crate::engine::parser::parse_document::<true>(text);
}

#[cfg(not(feature = "timing"))]
pub fn analyze(text: &[u8]) {
    std::hint::black_box(text);
}

// Сброс накопителя перед прогоном.
#[cfg(feature = "timing")]
pub fn reset() {
    ACCUM.with(|acc| acc.set(Timing::new()));
}

#[cfg(not(feature = "timing"))]
pub fn reset() {}

// Отчёт: ключи, отсортированные по числу операций.
#[cfg(feature = "timing")]
pub fn report() {
    let timing = ACCUM.with(|acc| acc.get());
    let mut rows: Vec<(TimingKey, u64, u64)> = timing
        .calls
        .iter()
        .enumerate()
        .map(|(i, &calls)| (ALL_KEYS[i], calls, timing.ops[i]))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));

    let total_ops: u64 = rows.iter().map(|r| r.2).sum();
    println!("\n── Внутренние операции по конструкциям ──");
    for (key, calls, ops) in rows {
        if calls == 0 && ops == 0 {
            continue;
        }
        let per = if calls > 0 {
            ops as f64 / calls as f64
        } else {
            0.0
        };
        let pct = if total_ops > 0 {
            ops as f64 / total_ops as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:>26} {:>12} вызовов {:>14} операций {:>6.2} оп/вызов {:>5.1}%",
            key.name(),
            fmt(calls),
            fmt(ops),
            per,
            pct
        );
    }
    println!(
        "{:>26} {:>12}    {:>14} операций",
        "ИТОГО",
        fmt(timing.calls.iter().sum::<u64>()),
        fmt(total_ops)
    );
}

#[cfg(not(feature = "timing"))]
pub fn report() {
    println!("\nПодсчёт операций выключен. Запусти с фичей: cargo bench --features timing");
}

// Число с разделителями тысяч: 65027295 → "65 027 295".
#[cfg(feature = "timing")]
fn fmt(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}
