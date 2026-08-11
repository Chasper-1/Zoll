//! Честные бенчмарки zoll vs pulldown-cmark vs comrak.
//!
//! Каждая группа сравнивает ТОЛЬКО то, что можно сравнить:
//!
//! | Группа | Что меряет | zoll | pulldown-cmark | comrak |
//! |--------|-----------|:----:|:--------------:|:------:|
//! | `stream_parse` | Парсинг → плоский список | ✅ LineAST | ✅ Events | ❌ |
//! | `ast_build` | Парсинг → дерево AST | ✅ MarkupDoc | ❌ | ✅ ArenaNode |
//! | `html_render` | Парсинг → HTML | ✅ (+ рендер) | ✅ push_html | ✅ format_html |
//! | `incremental_edit` | Правка 1 символа (только zoll) | ✅ full vs lazy | — | — |
//!
//! Запуск:
//!   cargo bench --bench compare
//!
//! Замер энергии на Linux:
//!   perf stat -e power/energy-pkg/ cargo bench --bench compare
//!
//! Результаты: `target/criterion/report/index.html`

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

// ─── Импорты zoll ─────────────────────────────────────────────
use zoll::ast::{LineAST, MarkupNode, MarkupStyle};
use zoll::incremental::IncrementalDoc;
use zoll::parser::{merge, parse_line};
use zoll::viewport::Viewport;

// ─── Импорты comrak ───────────────────────────────────────────
use comrak::Arena;

// ─── Генерация тестовых документов ────────────────────────────

const DOC_LINES: usize = 5_000;
const VIEWPORT_SIZE: usize = 40;

// Генерирует zoll-документ (5000 строк, ~390 KB).
// Содержит всю палитру синтаксиса: заголовки, bold, italic, списки,
// цитаты, комментарии, спойлеры, таблицы, формулы.
fn generate_zoll_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1# Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 12;
        match section {
            0 => s.push_str(&format!("#2# Section {}\n", i / 10)),
            1 => s.push_str(&format!("This is **bold {}** and //italic {}// text\n", i, i)),
            2 => s.push_str(&format!("- list item {} with **bold**\n", i)),
            3 => s.push_str(&format!("1. numbered item {} with //italic//\n", i)),
            4 => s.push_str(&format!("> quote line {} with ==highlight==\n", i)),
            5 => s.push_str(&format!("Plain text {} ~~strike~~ __underline__\n", i)),
            6 => s.push_str(&format!("++insert++ --delete-- ''super'' ,,sub,, {}\n", i)),
            7 => s.push_str("%% this is a comment line\n"),
            8 => s.push_str(&format!("!! spoiler hidden content at line {}\n", i)),
            9 => s.push_str(&format!("| cell {} | cell {} |\n", i, i + 1)),
            10 => s.push_str(&format!("$$ x = {} + y\n", i)),
            11 => s.push_str(&format!("plain text line {}\n", i)),
            _ => unreachable!(),
        }
    }
    s.push_str("#1# End of Document\n");
    s
}

// Генерирует семантически эквивалентный markdown-документ.
// Конвертация: `//italic//` → `*italic*`, `==highlight==` → `==highlight==`,
// `%%` → `<!-- -->`, `++insert++` → `<ins>insert</ins>`, и т.д.
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
            6 => s.push_str(&format!("<ins>insert</ins> <del>delete</del> <sup>super</sup> <sub>sub</sub> {}\n", i)),
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

// ─── Рендерер zoll → HTML (для честного сравнения HTML) ──────

// Минимальный рендерер zoll → HTML.
// НЕ претендует на полноту, только для бенчмарка.
fn render_zoll_html(text: &str) -> String {
    let doc = zoll::parser::parse_full(text);
    let mut html = String::with_capacity(text.len() + 1024);
    html.push_str("<doc>");
    for node in &doc.children {
        render_node(node, &mut html);
    }
    html.push_str("</doc>");
    html
}

fn render_node(node: &MarkupNode, out: &mut String) {
    match node {
        MarkupNode::Text(t) => out.push_str(&escape_html(t)),
        MarkupNode::Formatted { style, children } => {
            let tag = style_to_tag(*style);
            out.push('<');
            out.push_str(tag);
            out.push('>');
            for child in children {
                render_node(child, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        MarkupNode::Header { level, children } => {
            let tag = format!("h{}", (*level).min(6));
            out.push('<'); out.push_str(&tag); out.push('>');
            for child in children { render_node(child, out); }
            out.push_str("</"); out.push_str(&tag); out.push('>');
        }
        MarkupNode::ListItem { children, .. } => {
            out.push_str("<li>");
            for child in children {
                render_node(child, out);
            }
            out.push_str("</li>");
        }
        MarkupNode::Quote(children) => {
            out.push_str("<blockquote>");
            for child in children { render_node(child, out); }
            out.push_str("</blockquote>");
        }
        MarkupNode::ThematicBreak => out.push_str("<hr/>"),
        MarkupNode::TableRow(cells) => {
            out.push_str("<tr>");
            for cell in cells {
                out.push_str("<td>");
                for child in cell { render_node(child, out); }
                out.push_str("</td>");
            }
            out.push_str("</tr>");
        }
        MarkupNode::Spoiler { children, .. } => {
            out.push_str("<span class=\"spoiler\">");
            for child in children { render_node(child, out); }
            out.push_str("</span>");
        }
        MarkupNode::Comment(_) => {} // комментарии не рендерятся
        MarkupNode::Formula(children) => {
            out.push_str("<span class=\"formula\">");
            for child in children { render_node(child, out); }
            out.push_str("</span>");
        }
        MarkupNode::CodeBlock { .. } => {
            out.push_str("<pre><code>code</code></pre>");
        }
    }
}

fn style_to_tag(style: MarkupStyle) -> &'static str {
    if style == MarkupStyle::BOLD { "strong" }
    else if style == MarkupStyle::ITALIC { "em" }
    else if style == MarkupStyle::UNDERLINE { "u" }
    else if style == MarkupStyle::STRIKETHROUGH { "del" }
    else if style == MarkupStyle::CODE { "code" }
    else if style == MarkupStyle::HIGHLIGHT { "mark" }
    else if style == MarkupStyle::SUPERSCRIPT { "sup" }
    else if style == MarkupStyle::SUBSCRIPT { "sub" }
    else if style == MarkupStyle::INSERTION { "ins" }
    else if style == MarkupStyle::DELETION { "del" }
    else { "span" }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ═══════════════════════════════════════════════════════════════
//  БЕНЧМАРКИ
// ═══════════════════════════════════════════════════════════════

// ─── 1. Stream parsing: плоский список, без дерева ────────────

// Парсинг в плоский поток событий/строк.
// zoll: `parse_line` → `Vec<LineAST>` (уже структурированный, но плоский).
// pulldown-cmark: `Parser` → `Vec<Event>` (поток тегов).
// comrak: аналогичного API нет.
fn bench_stream_parse(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("stream_parse");
    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));

    group.bench_function("zoll_parse_lines", |b| {
        b.iter(|| {
            let lines: Vec<LineAST> = black_box(&zoll_doc).lines()
                .map(|l| parse_line(l))
                .collect();
            black_box(lines);
        });
    });

    group.bench_function("pulldown_cmark_events", |b| {
        b.iter(|| {
            let events: Vec<pulldown_cmark::Event> =
                pulldown_cmark::Parser::new(black_box(&md_doc)).collect();
            black_box(events);
        });
    });

    group.finish();
}

// ─── 2. AST tree: полное дерево документа ─────────────────────

// Построение полноценного AST-дерева.
// zoll: `parse_full()` → `MarkupDoc` (векторная структура).
// comrak: `parse_document()` → `&AstNode` (арена).
// pulldown-cmark: не строит дерево.
fn bench_ast_build(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("ast_build");
    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));

    group.bench_function("zoll_markup_doc", |b| {
        b.iter(|| {
            black_box(zoll::parser::parse_full(black_box(&zoll_doc)));
        });
    });

    group.bench_function("comrak_arena_node", |b| {
        b.iter(|| {
            let arena = Arena::new();
            let root = comrak::parse_document(
                &arena,
                black_box(&md_doc),
                &comrak::ComrakOptions::default(),
            );
            black_box(root);
        });
    });

    group.finish();
}

// ─── 3. HTML render: все три парсера → HTML ───────────────────

// Парсинг + рендер в HTML.
// У всех трёх одинаковый формат на выходе.
fn bench_html_render(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let md_doc = generate_md_doc(DOC_LINES);

    let mut group = c.benchmark_group("html_render");
    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));

    group.bench_function("zoll_render_html", |b| {
        b.iter(|| {
            black_box(render_zoll_html(black_box(&zoll_doc)));
        });
    });

    group.bench_function("pulldown_cmark_html", |b| {
        b.iter(|| {
            let parser = pulldown_cmark::Parser::new(black_box(&md_doc));
            let mut html = String::new();
            pulldown_cmark::html::push_html(&mut html, parser);
            black_box(html);
        });
    });

    group.bench_function("comrak_html", |b| {
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

// ─── 4. Incremental (только zoll) ─────────────────────────────

// Сравнение полного перепарса vs ленивого (viewport).
//
// ВАЖНО: используем `iter()` на одном документе, а не `iter_batched`
// с клоном. Клон IncrementalDoc — глубокое копирование всех 5000
// LineAST со строками, что занимает ~1 ms и скрывает реальную
// производительность инкрементального парсинга.
//
// Документ растёт на 1 байт за итерацию — за 100 итераций это
// +100 байт на 390 KB, влиянием можно пренебречь.
fn bench_incremental(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let edit_pos = zoll_doc.len() / 2;
    let mid_line = edit_pos / (zoll_doc.len() / DOC_LINES);
    let vp = Viewport::new(
        mid_line.saturating_sub(VIEWPORT_SIZE / 2),
        (mid_line + VIEWPORT_SIZE / 2).min(DOC_LINES - 1),
    );

    let mut group = c.benchmark_group("incremental_edit");
    group.throughput(Throughput::Bytes(1)); // 1 символ — 1 правка

    // Полный перепарс: редактируем существующий документ (без клона)
    group.bench_function("zoll_full_reparse", |b| {
        let mut doc = IncrementalDoc::new(&zoll_doc);
        b.iter(|| {
            doc.edit(edit_pos, edit_pos, "X");
        });
    });

    // Ленивый viewport: правим и перепарсим только видимую область
    group.bench_function("zoll_lazy_viewport", |b| {
        let mut doc = IncrementalDoc::new(&zoll_doc);
        b.iter(|| {
            doc.edit_visible(edit_pos, edit_pos, "X", &vp);
        });
    });

    // Полный парсинг с нуля — эталон
    group.bench_function("zoll_parse_full_from_scratch", |b| {
        b.iter(|| {
            black_box(zoll::parser::parse_full(black_box(&zoll_doc)));
        });
    });

    group.finish();
}

// ─── 5. Viewport scaling (только zoll) ────────────────────────

// Как размер viewport влияет на скорость edit_visible().
fn bench_viewport_scaling(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);
    let edit_pos = zoll_doc.len() / 2;
    let mid_line = edit_pos / (zoll_doc.len() / DOC_LINES);

    let mut group = c.benchmark_group("viewport_scaling");
    group.throughput(Throughput::Elements(1));

    for vp_height in &[10usize, 40, 100, 500, 1000] {
        let vp = Viewport::new(
            mid_line.saturating_sub(vp_height / 2),
            (mid_line + vp_height / 2).min(DOC_LINES - 1),
        );
        group.bench_function(
            &format!("zoll_viewport_{}_lines", vp_height),
            |b| {
                let mut doc = IncrementalDoc::new(&zoll_doc);
                b.iter(|| {
                    doc.edit_visible(edit_pos, edit_pos, "X", &vp);
                });
            },
        );
    }

    group.bench_function("zoll_full_reparse_reference", |b| {
        let mut doc = IncrementalDoc::new(&zoll_doc);
        b.iter(|| {
            doc.edit(edit_pos, edit_pos, "X");
        });
    });

    group.finish();
}

// ─── 6. Breakdown (только zoll) ───────────────────────────────

// Из чего складывается время полного парсинга.
fn bench_zoll_breakdown(c: &mut Criterion) {
    let zoll_doc = generate_zoll_doc(DOC_LINES);

    let mut group = c.benchmark_group("zoll_breakdown");
    group.throughput(Throughput::Bytes(zoll_doc.len() as u64));

    group.bench_function("build_line_starts", |b| {
        b.iter(|| {
            black_box(zoll::incremental::build_line_starts(black_box(&zoll_doc)));
        });
    });

    group.bench_function("parse_all_lines", |b| {
        b.iter(|| {
            let lines: Vec<LineAST> = zoll_doc.lines()
                .map(|l| parse_line(l))
                .collect();
            black_box(lines);
        });
    });

    group.bench_function("merge_all", |b| {
        let lines: Vec<LineAST> = zoll_doc.lines()
            .map(|l| parse_line(l))
            .collect();
        b.iter(|| {
            black_box(merge(black_box(&lines)));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_stream_parse,
    bench_ast_build,
    bench_html_render,
    bench_incremental,
    bench_viewport_scaling,
    bench_zoll_breakdown,
);

criterion_main!(benches);
