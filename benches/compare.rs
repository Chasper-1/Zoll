//! Бенчмарки: сравнение zoll с pulldown-cmark и comrak.
//!
//! Запуск:
//!   cargo bench --bench compare
//!
//! Замер энергии на Linux:
//!   perf stat -e power/energy-pkg/ cargo bench --bench compare
//!
//! Результаты: target/criterion/report/index.html

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

// ─── Импорты zoll ─────────────────────────────────────────────
use zoll::incremental::IncrementalDoc;
use zoll::parser::parse_full;
use zoll::viewport::Viewport;

// ─── Генерация тестовых документов ────────────────────────────

/// Генерирует zoll-документ размером `lines` строк.
/// Содержит: заголовки, bold, italic, списки, цитаты, комментарии.
fn generate_zoll_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1# Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 10;
        match section {
            0 => s.push_str(&format!("#2# Section {}\n", i / 10)),
            1 => s.push_str(&format!("This is **bold {}** and //italic {}// text\n", i, i)),
            2 => s.push_str(&format!("- list item {} with **bold**\n", i)),
            3 => s.push_str(&format!("1. numbered item {} with //italic//\n", i)),
            4 => s.push_str(&format!("> quote line {} with ==highlight==\n", i)),
            5 => s.push_str(&format!("Plain text line {} with ~~strike~~\n", i)),
            6 => s.push_str(&format!("Line with ++insert++ and --delete-- {}\n", i)),
            7 => s.push_str(&format!("Text with __underline__ and ''super'' {}\n", i)),
            8 => s.push_str("%% this is a comment line\n"),
            9 => s.push_str(&format!("!! spoiler with hidden content at line {}\n", i)),
            _ => s.push_str(&format!("default line {}\n", i)),
        }
    }
    s.push_str("#1# End of Document\n");
    s
}

/// Генерирует эквивалентный markdown-документ.
/// Конвертирует zoll-синтаксис в markdown-совместимый.
fn generate_md_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("# Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 10;
        match section {
            0 => s.push_str(&format!("## Section {}\n", i / 10)),
            1 => s.push_str(&format!("This is **bold {}** and *italic {}* text\n", i, i)),
            2 => s.push_str(&format!("- list item {} with **bold**\n", i)),
            3 => s.push_str(&format!("1. numbered item {} with *italic*\n", i)),
            4 => s.push_str(&format!("> quote line {} with `highlight`\n", i)),
            5 => s.push_str(&format!("Plain text line {} with ~~strike~~\n", i)),
            6 => s.push_str(&format!("Line with <ins>insert</ins> and <del>delete</del> {}\n", i)),
            7 => s.push_str(&format!("Text with <u>underline</u> and <sup>super</sup> {}\n", i)),
            8 => s.push_str("<!-- this is a comment line -->\n"),
            9 => s.push_str(&format!("||spoiler with hidden content at line {}||\n", i)),
            _ => s.push_str(&format!("default line {}\n", i)),
        }
    }
    s.push_str("# End of Document\n");
    s
}

// ─── Бенчмарки ───────────────────────────────────────────────

const DOC_LINES: usize = 5_000;   // ~390 КБ
const VIEWPORT_SIZE: usize = 40; // типичный размер экрана

fn bench_full_parse(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("full_parse");
    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));

    group.bench_function("zoll_parse_full", |b| {
        b.iter(|| {
            black_box(parse_full(black_box(&zoll_doc)));
        });
    });

    group.bench_function("pulldown_cmark", |b| {
        b.iter(|| {
            let parser = pulldown_cmark::Parser::new(black_box(&md_doc));
            let mut html = String::new();
            pulldown_cmark::html::push_html(&mut html, parser);
            black_box(html);
        });
    });

    group.bench_function("comrak", |b| {
        b.iter(|| {
            let result = comrak::markdown_to_html(
                black_box(&md_doc),
                &comrak::ComrakOptions::default(),
            );
            black_box(result);
        });
    });

    group.finish();
}

fn bench_incremental_edit(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    // Позиция правки — примерно посередине документа
    let edit_pos = zoll_doc.len() / 2;

    let mut group = c.benchmark_group("incremental_edit");
    group.throughput(Throughput::Bytes(1)); // 1 символ — 1 правка

    // Zoll: полный перепарс (edit без вьюпорта)
    group.bench_function("zoll_full_reparse", |b| {
        b.iter_batched(
            || IncrementalDoc::new(&zoll_doc),
            |mut doc| { doc.edit(edit_pos, edit_pos, "X"); },
            criterion::BatchSize::SmallInput,
        );
    });

    // Zoll: ленивый перепарс (edit_visible с вьюпортом)
    group.bench_function("zoll_lazy_viewport", |b| {
        let vp = Viewport::new(
            edit_pos / (zoll_doc.len() / DOC_LINES) - VIEWPORT_SIZE / 2,
            edit_pos / (zoll_doc.len() / DOC_LINES) + VIEWPORT_SIZE / 2,
        );
        b.iter_batched(
            || IncrementalDoc::new(&zoll_doc),
            |mut doc| { doc.edit_visible(edit_pos, edit_pos, "X", &vp); },
            criterion::BatchSize::SmallInput,
        );
    });

    // Markdown: парсинг всего документа (альтернативы инкременту нет)
    group.bench_function("pulldown_cmark_reparse", |b| {
        b.iter_batched(
            || md_doc.clone(),
            |doc| {
                let parser = pulldown_cmark::Parser::new(&doc);
                let mut html = String::new();
                pulldown_cmark::html::push_html(&mut html, parser);
                black_box(html);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_latency(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("latency");
    group.throughput(Throughput::Elements(1));
    group.sample_size(10);

    // Время до первого результата: zoll даёт line_ast построчно,
    // pulldown-cmark/comrak — streaming
    group.bench_function("zoll_first_line", |b| {
        b.iter(|| {
            let doc = IncrementalDoc::new(black_box(&zoll_doc));
            black_box(&doc.line_asts[0]);
        });
    });

    group.bench_function("pulldown_cmark_first_event", |b| {
        b.iter(|| {
            let parser = pulldown_cmark::Parser::new(black_box(&md_doc));
            for event in parser {
                black_box(event);
                break; // только первый
            }
        });
    });

    group.finish();
}

fn bench_viewport_vs_full(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let edit_pos = zoll_doc.len() / 2;
    let mid_line = edit_pos / (zoll_doc.len() / DOC_LINES);

    let mut group = c.benchmark_group("viewport_scaling");
    group.throughput(Throughput::Elements(1));

    for vp_height in &[10usize, 40, 100, 500, 1000] {
        let vp_height = *vp_height;
        let vp = Viewport::new(
            mid_line.saturating_sub(vp_height / 2),
            (mid_line + vp_height / 2).min(DOC_LINES - 1),
        );

        group.bench_function(
            &format!("zoll_viewport_{}_lines", vp_height),
            |b| {
                b.iter_batched(
                    || IncrementalDoc::new(&zoll_doc),
                    |mut doc| { doc.edit_visible(edit_pos, edit_pos, "X", &vp); },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    // Для сравнения: полный перепарс
    group.bench_function("zoll_full_reparse_ref", |b| {
        b.iter_batched(
            || IncrementalDoc::new(&zoll_doc),
            |mut doc| { doc.edit(edit_pos, edit_pos, "X"); },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_full_parse,
    bench_incremental_edit,
    bench_latency,
    bench_viewport_vs_full,
);

criterion_main!(benches);
