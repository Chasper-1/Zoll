//! Диагностика производительности: время по каждому синтаксическому приколу.
//!
//! Включается фичей `timing` (cargo bench --features timing). Когда фича
//! выключена — все функции no-op, компилятор выкидывает их в ноль, оверхеда
//! в проде нет.
//!
//! Как читать отчёт: время — это сумма по всем вызовам `process_marker`,
//! распределённая по конструкциям. Проценты показывают, какой прикол жрёт
//! больше всего времени. Абсолютные цифры с включённым таймингом завышены
//! (~20-30 нс на маркер), но распределение честное.

use crate::engine::SyntaxKind;
use std::cell::Cell;
#[cfg(feature = "timing")]
use std::time::Instant;

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

// Накопитель: счётчик и наносекунды на каждый ключ.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub counts: [u64; TimingKey::COUNT],
    pub nanos: [u128; TimingKey::COUNT],
}

impl Timing {
    pub const fn new() -> Self {
        Timing {
            counts: [0; TimingKey::COUNT],
            nanos: [0; TimingKey::COUNT],
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
}

// Таймер одного маркера. Ключ назначается внутри ветки через `set`.
// При выходе из функции (включая ранние return) время пишется в свой ключ.
pub struct MarkerTimer {
    #[cfg(feature = "timing")]
    key: Cell<TimingKey>,
    #[cfg(feature = "timing")]
    start: Instant,
}

#[cfg(feature = "timing")]
pub fn begin() -> MarkerTimer {
    MarkerTimer {
        key: Cell::new(TimingKey::Ignored),
        start: Instant::now(),
    }
}

#[cfg(not(feature = "timing"))]
pub fn begin() -> MarkerTimer {
    MarkerTimer {}
}

impl MarkerTimer {
    #[cfg(feature = "timing")]
    pub fn set(&self, key: TimingKey) {
        self.key.set(key);
    }

    #[cfg(not(feature = "timing"))]
    pub fn set(&self, key: TimingKey) {
        std::hint::black_box(key);
    }
}

#[cfg(feature = "timing")]
impl Drop for MarkerTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_nanos();
        let idx = self.key.get().index();
        ACCUM.with(|acc| {
            let mut t = acc.get();
            t.counts[idx] += 1;
            t.nanos[idx] += elapsed;
            acc.set(t);
        });
    }
}

// Сброс накопителя перед прогоном.
#[cfg(feature = "timing")]
pub fn reset() {
    ACCUM.with(|acc| acc.set(Timing::new()));
}

#[cfg(not(feature = "timing"))]
pub fn reset() {}

// Отчёт: ключи, отсортированные по времени.
#[cfg(feature = "timing")]
pub fn report() {
    let timing = ACCUM.with(|acc| acc.get());
    let mut rows: Vec<(TimingKey, u64, u128)> = timing
        .counts
        .iter()
        .enumerate()
        .map(|(i, &count)| (ALL_KEYS[i], count, timing.nanos[i]))
        .collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.2));

    let total: u128 = rows.iter().map(|r| r.2).sum();
    println!("\n── Тайминг по конструкциям ──");
    for (key, count, nanos) in rows {
        if count == 0 && nanos == 0 {
            continue;
        }
        let pct = if total > 0 {
            nanos as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:>26} {:>8} шт {:>10.1} µs {:>5.1}%",
            key.name(),
            count,
            nanos as f64 / 1e3,
            pct
        );
    }
    println!(
        "{:>26} {:>8}    {:>10.1} µs",
        "ИТОГО",
        timing.counts.iter().sum::<u64>(),
        total as f64 / 1e3
    );
}

#[cfg(not(feature = "timing"))]
pub fn report() {
    println!("\nТайминг выключен. Запусти с фичей: cargo bench --features timing");
}
