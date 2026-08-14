//! Бенчмарки zoll (движок) vs pulldown-cmark.
//!
//! | Группа | Что меряет | zoll | pulldown-cmark |
//! |--------|-----------|:----:|:--------------:|
//! | `parse_spans` | Парсинг → плоский список | ✅ Vec<SyntaxSpan> | ✅ Vec<Event> |
//! | `zoll_breakdown` | Этапы движка (scan/collect/resolve) | ✅ | — |
//!
//! Запуск:
//!   cargo bench --bench compare
//!
//! Результаты: `target/criterion/report/index.html`

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};

use zoll::engine::{Engine, INTERESTING_BYTES, collect, resolve, scan};

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

// ═══════════════════════════════════════════════════════════════
//  БЕНЧМАРКИ
// ═══════════════════════════════════════════════════════════════

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
            black_box(&engine.spans);
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

    group.finish();
}

// ─── 3. Breakdown: этапы движка zoll ──────────────────────────
//
// Из чего складывается время полного парсинга.
fn bench_zoll_breakdown(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let text = zoll_doc.as_bytes();

    let mut group = c.benchmark_group("zoll_breakdown");
    group.throughput(Throughput::Bytes(text.len() as u64));

    group.bench_function("scan_simd", |b| {
        b.iter(|| {
            black_box(scan(black_box(text), INTERESTING_BYTES));
        });
    });

    // Сколько стоит владение буфером: копия документа в Engine::parse.
    group.bench_function("copy_buffer", |b| {
        b.iter(|| {
            black_box(black_box(text).to_vec());
        });
    });

    group.bench_function("build_line_map", |b| {
        let events = scan(text, INTERESTING_BYTES);
        b.iter(|| {
            black_box(zoll::engine::LineMap::from_events(black_box(&events)));
        });
    });

    group.bench_function("collect_markers", |b| {
        let events = scan(text, INTERESTING_BYTES);
        b.iter(|| {
            black_box(collect(black_box(&events)));
        });
    });

    group.bench_function("resolve_spans", |b| {
        let events = scan(text, INTERESTING_BYTES);
        let line_map = zoll::engine::LineMap::from_events(&events);
        let markers = collect(&events);
        b.iter(|| {
            black_box(resolve(black_box(text), black_box(&markers), &line_map));
        });
    });

    group.bench_function("build_dependency_graph", |b| {
        let engine = Engine::parse(text);
        let spans = &engine.spans;
        b.iter(|| {
            black_box(zoll::engine::DependencyGraph::new(black_box(spans)));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_spans,
    bench_html_render,
    bench_zoll_breakdown,
);

criterion_main!(benches);
