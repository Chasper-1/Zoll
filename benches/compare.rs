//! Бенчмарки zoll (движок) vs другие Rust markdown-парсеры.
//!
//! | Группа | Что меряет | zoll | pulldown-cmark | sparkdown | ferromark |
//! |--------|-----------|:----:|:--------------:|:---------:|:---------:|
//! | `parse_spans` | Парсинг → плоский список | ✅ Vec<SyntaxSpan> | ✅ Vec<Event> | — | ✅ Vec<BlockEvent> |
//! | `html_render` | Парсинг + рендер в HTML | — | ✅ | ✅ | ✅ |
//! | `zoll_breakdown` | scan (маски) vs полный проход | ✅ | — | — | — |
//! | `full_markup` | Полная палитра конструкций (вкл. блочные) | ✅ | ✅ | — | ✅ |
//!
//! sparkdown (0.1.0) — HTML-only (scaffold, только абзацы), поэтому только
//! в `html_render`. ferromark — стриминг событий без HTML (BlockParser),
//! поэтому и в `parse_spans`, и в `html_render`. Оба парсят тот же `md_doc`,
//! что и pulldown-cmark.
//!
//! Запуск:
//!   cargo bench --bench compare
//!
//! Результаты: `target/criterion/report/index.html`

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};

use zoll::engine::{Engine, INTERESTING_BYTES, scan};

// ─── Генерация тестовых документов ────────────────────────────

const DOC_LINES: usize = 5_000;

// Генерирует zoll-документ (5000 строк, ~390 KB).
// Содержит всю палитру синтаксиса: заголовки, bold, italic, списки,
// цитаты, комментарии, спойлеры, таблицы, формулы.
fn generate_zoll_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1 Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 12;
        match section {
            0 => s.push_str(&format!("#2 Section {}\n", i / 10)),
            1 => s.push_str(&format!(
                "This is **bold {})) and //italic {})) text\n",
                i, i
            )),
            2 => s.push_str(&format!("- list item {} with **bold))\n", i)),
            3 => s.push_str(&format!("1. numbered item {} with //italic))\n", i)),
            4 => s.push_str(&format!("> quote line {} with ==highlight))\n", i)),
            5 => s.push_str(&format!("Plain text {} ~~strike)) __underline))\n", i)),
            6 => s.push_str(&format!("++insert)) --delete)) ''super)) ,,sub)) {}\n", i)),
            7 => s.push_str(&format!("visible text %%comment {}}}\n", i)),
            8 => s.push_str(&format!("text !!spoiler hidden {}}}\n", i)),
            9 => s.push_str(&format!("| cell {} | cell {} |\n", i, i + 1)),
            10 => s.push_str(&format!("x = {} $$sqrt({})}}\n", i, i)),
            11 => s.push_str(&format!("plain text line {}\n", i)),
            _ => unreachable!(),
        }
    }
    s.push_str("#1 End of Document\n");
    s
}

// Генерирует семантически эквивалентный markdown-документ.
fn generate_md_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("# Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 12;
        match section {
            0 => s.push_str(&format!("## Section {}\n", i / 10)),
            1 => s.push_str(&format!("This is **bold {}** and *italic {}* text\n", i, i)),
            2 => s.push_str(&format!("- list item {} with **bold**\n", i)),
            3 => s.push_str(&format!("1. numbered item {} with *italic*\n", i)),
            4 => s.push_str(&format!("> quote line {} with ==highlight==\n", i)),
            5 => s.push_str(&format!("Plain text {} ~~strike~~ <u>underline</u>\n", i)),
            6 => s.push_str(&format!(
                "<ins>insert</ins> <del>delete</del> <sup>super</sup> <sub>sub</sub> {}\n",
                i
            )),
            7 => s.push_str("<!-- this is a comment line -->\n"),
            8 => s.push_str(&format!("||spoiler hidden content at line {}||\n", i)),
            9 => s.push_str(&format!("| cell {} | cell {} |\n", i, i + 1)),
            10 => s.push_str(&format!("$$ x = {} + y $$\n", i)),
            11 => s.push_str(&format!("plain text line {}\n", i)),
            _ => unreachable!(),
        }
    }
    s.push_str("# End of Document\n");
    s
}

// Генерирует zoll-документ с ПОЛНОЙ палитрой конструкций: все inline
// (bold/italic/underline/strike/highlight/insert/delete/super/sub/formula),
// все line-level (%%/$$/!! + спойлер с заголовком), все block-level
// (%%%/$$$/!!!), структура (#N, #:тег, ---, списки, цитата, таблица).
// Цикл из 16 секций: 13 однострочных + 3 блочных (по 3 строки) = 22 строки.
fn generate_full_markup_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1 Full Markup Benchmark\n\n");
    let cycles = lines * 16 / 22;
    for i in 0..cycles {
        let section = i % 16;
        match section {
            0 => s.push_str(&format!("#2 Section {}\n", i)),
            1 => s.push_str(&format!(
                "**bold {})) //italic {})) __underline{})) ~~strike{}))\n",
                i, i, i, i
            )),
            2 => s.push_str(&format!(
                "==highlight{})) ++insert{})) --delete{})) ''super{})) ,,sub{})) $x_{}))\n",
                i, i, i, i, i, i
            )),
            3 => s.push_str(&format!("- list item {} with **bold))\n", i)),
            4 => s.push_str(&format!("1. numbered item {} with //italic))\n", i)),
            5 => s.push_str(&format!("> quote line {} with ==highlight))\n", i)),
            6 => s.push_str(&format!("#:tag{}\n", i)),
            7 => s.push_str("---\n"),
            8 => s.push_str(&format!("| cell {} | cell {} |\n", i, i + 1)),
            9 => s.push_str(&format!("visible text %%comment {}}}\n", i)),
            10 => s.push_str(&format!("x = {} $$sqrt({})}}\n", i, i)),
            11 => s.push_str(&format!("text !!spoiler {}}}\n", i)),
            12 => s.push_str(&format!("!!заголовок: скрытое {}}}\n", i)),
            13 => s.push_str(&format!("%%%\nblock comment {}\n}}\n", i)),
            14 => s.push_str(&format!("$$$\nblock formula {}\n}}\n", i)),
            15 => s.push_str(&format!("!!!спойлер:\nblock spoiler {}\n}}\n", i)),
            _ => unreachable!(),
        }
    }
    s.push_str("#1 End of Document\n");
    s
}

// Markdown-эквивалент полной палитры (для сравнения с другими парсерами).
fn generate_full_markup_md(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("# Full Markup Benchmark\n\n");
    let cycles = lines * 16 / 22;
    for i in 0..cycles {
        let section = i % 16;
        match section {
            0 => s.push_str(&format!("## Section {}\n", i)),
            1 => s.push_str(&format!(
                "**bold {}** *italic {}* <u>underline {}</u> ~~strike {}~~\n",
                i, i, i, i
            )),
            2 => s.push_str(&format!(
                "<mark>highlight {}</mark> <ins>insert {}</ins> <del>delete {}</del> \
                 <sup>super {}</sup> <sub>sub {}</sub> $x_{}$\n",
                i, i, i, i, i, i
            )),
            3 => s.push_str(&format!("- list item {} with **bold**\n", i)),
            4 => s.push_str(&format!("1. numbered item {} with *italic*\n", i)),
            5 => s.push_str(&format!("> quote line {} with ==highlight==\n", i)),
            6 => s.push_str(&format!("<!-- tag{} -->\n", i)),
            7 => s.push_str("---\n"),
            8 => s.push_str(&format!("| cell {} | cell {} |\n", i, i + 1)),
            9 => s.push_str("<!-- comment -->\n"),
            10 => s.push_str(&format!("$$ x = {} + y $$\n", i)),
            11 => s.push_str(&format!("||spoiler hidden {}||\n", i)),
            12 => s.push_str(&format!("||spoiler title: hidden {}||\n", i)),
            13 => s.push_str(&format!("<!--\nblock comment {}\n-->\n", i)),
            14 => s.push_str(&format!("$$\nblock formula {}\n$$\n", i)),
            15 => s.push_str(&format!("||\nblock spoiler {}\n||\n", i)),
            _ => unreachable!(),
        }
    }
    s.push_str("# End of Document\n");
    s
}

// ═══════════════════════════════════════════════════════════════
//  БЕНЧМАРКИ
// ═══════════════════════════════════════════════════════════════

// Функция для вывода размера документов в байтах перед бенчмарками, чтобы не тормозить и не искажать результаты.
fn len_parse_spans(_c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);
    let full_zoll = generate_full_markup_doc(DOC_LINES);
    let full_md = generate_full_markup_md(DOC_LINES);

    eprintln!("Zoll:        {} bytes", zoll_doc.len());
    eprintln!("MD:          {} bytes", md_doc.len());
    eprintln!("Zoll full:   {} bytes", full_zoll.len());
    eprintln!("MD full:     {} bytes", full_md.len());
}

// ─── 1. Парсинг в плоский список ──────────────────────────────
//
// zoll: `Engine::parse` → `Vec<SyntaxSpan>` (байтовые диапазоны).
// pulldown-cmark: `Parser` → `Vec<Event>` (поток тегов).
// Оба — плоский поток без дерева, сравнение честное.
//
// Методика (чтобы нельзя было докопаться до объективности):
// - каждый парсер парсит СВОЙ документ (zoll-синтаксис vs семантически
//   эквивалентный CommonMark) — разных текстов для обоих не бывает,
//   т.к. языки различаются;
// - throughput считается ПО СВОЕМУ входу: zoll — по zoll_doc, pulldown —
//   по md_doc. Раньше pulldown мерили байтами zoll-документа — нечестно;
// - главная метрика — абсолютное время, throughput в MiB/s — производная
//   от размера входа каждого парсера.
fn bench_parse_spans(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("parse_spans");

    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));
    group.bench_function("zoll_engine_parse", |b| {
        b.iter(|| {
            let engine = Engine::parse(black_box(zoll_doc.as_bytes()));
            black_box(engine.spans());
        });
    });

    group.throughput(Throughput::Bytes(md_doc.len() as u64));
    group.bench_function("pulldown_cmark_events", |b| {
        b.iter(|| {
            let events: Vec<pulldown_cmark::Event> =
                pulldown_cmark::Parser::new(black_box(&md_doc)).collect();
            black_box(events);
        });
    });

    // ferromark: стриминг блочных событий без HTML (BlockParser → Vec<BlockEvent>).
    // Options::commonmark() — синтаксис CommonMark; render_policy на парсинг не влияет.
    group.bench_function("ferromark_block_events", |b| {
        b.iter(|| {
            let mut parser = ferromark::block::BlockParser::new_with_options(
                black_box(md_doc.as_bytes()),
                ferromark::Options::commonmark(),
            );
            let mut events: Vec<ferromark::block::BlockEvent> = Vec::new();
            parser.parse(&mut events);
            black_box(events);
        });
    });

    group.finish();
}

// ─── 2. Парсинг + рендер в HTML ───────────────────────────────
//
// pulldown-cmark умеет рендерить в HTML нативно.
// zoll: движок отдаёт спаны, рендеринг — задача редактора, поэтому
// честного сравнения HTML здесь нет; пулдаун рендерим для ориентира.
fn bench_html_render(c: &mut Criterion) {
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("html_render");
    group.throughput(Throughput::Bytes(md_doc.len() as u64));

    group.bench_function("pulldown_cmark_html", |b| {
        b.iter(|| {
            let parser = pulldown_cmark::Parser::new(black_box(&md_doc));
            let mut html = String::new();
            pulldown_cmark::html::push_html(&mut html, parser);
            black_box(html);
        });
    });

    // sparkdown: CommonMark 0.31.2, дефолтный быстрый путь (без фич).
    // Внимание: 0.1.0 — scaffold, реально парсит только абзацы.
    group.bench_function("sparkdown_html", |b| {
        b.iter(|| {
            let html = sparkdown::to_html(black_box(&md_doc));
            black_box(html);
        });
    });

    // ferromark: CommonMark-синтаксис, Trusted-рендер (raw HTML пропускается,
    // как у pulldown; дефолтный Untrusted экранировал бы его — нечестно).
    let ferromark_opts = ferromark::Options {
        render_policy: ferromark::RenderPolicy::Trusted,
        ..ferromark::Options::commonmark()
    };
    group.bench_function("ferromark_html", |b| {
        b.iter(|| {
            let html = ferromark::to_html_with_options(black_box(&md_doc), &ferromark_opts);
            black_box(html);
        });
    });

    group.finish();
}

// ─── 3. Breakdown движка zoll ─────────────────────────────────
//
// SIMD-скан (маски) против полного прохода — из чего складывается время.
fn bench_zoll_breakdown(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let text = zoll_doc.as_bytes();

    let mut group = c.benchmark_group("zoll_breakdown");
    group.throughput(Throughput::Bytes(text.len() as u64));

    // Только SIMD-скан: маски никуда не складываются.
    group.bench_function("scan_masks", |b| {
        b.iter(|| {
            let mut blocks = 0u32;
            scan(black_box(text), INTERESTING_BYTES, |_, _| blocks += 1);
            black_box(blocks);
        });
    });

    // Полный парсинг: scan → маркеры → конструкции + карта строк + граф.
    group.bench_function("engine_parse", |b| {
        b.iter(|| {
            let engine = Engine::parse(black_box(text));
            black_box(engine.spans());
        });
    });

    group.finish();
}

// ─── 4. Полная палитра конструкций ────────────────────────────
//
// Отдельный документ со ВСЕМИ конструкциями языка (включая блочные
// %%%/$$$/!!!): позволяет сравнивать парсеры на полной разметке и знать
// время для документа того же размера в строках, но с полной разметкой.
fn bench_full_markup(c: &mut Criterion) {
    let zoll_doc = generate_full_markup_doc(DOC_LINES);
    let md_doc = generate_full_markup_md(DOC_LINES);

    let mut group = c.benchmark_group("full_markup");

    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));
    group.bench_function("zoll_engine_parse", |b| {
        b.iter(|| {
            let engine = Engine::parse(black_box(zoll_doc.as_bytes()));
            black_box(engine.spans());
        });
    });

    group.throughput(Throughput::Bytes(md_doc.len() as u64));
    group.bench_function("pulldown_cmark_events", |b| {
        b.iter(|| {
            let events: Vec<pulldown_cmark::Event> =
                pulldown_cmark::Parser::new(black_box(&md_doc)).collect();
            black_box(events);
        });
    });

    group.bench_function("ferromark_block_events", |b| {
        b.iter(|| {
            let mut parser = ferromark::block::BlockParser::new_with_options(
                black_box(md_doc.as_bytes()),
                ferromark::Options::commonmark(),
            );
            let mut events: Vec<ferromark::block::BlockEvent> = Vec::new();
            parser.parse(&mut events);
            black_box(events);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    len_parse_spans,
    bench_parse_spans,
    bench_zoll_breakdown,
    bench_full_markup,
    bench_html_render,
);

criterion_main!(benches);
