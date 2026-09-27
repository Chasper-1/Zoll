//! Бенчмарки zoll (движок) vs другие Rust markdown-парсеры.
//!
//! | Группа | Что меряет | zoll | pulldown-cmark | sparkdown | ferromark |
//! |--------|-----------|:----:|:--------------:|:---------:|:---------:|
//! | `parse_spans` | Файл → структура, готовая к рендеру | ✅ Vec<SyntaxSpan> | ✅ Vec<Event> | — | ✅ Vec<BlockEvent> + Vec<InlineEvent> |
//! | `html_render` | Файл → HTML (финальный формат) | — | ✅ | ✅ | ✅ |
//! | `zoll_breakdown` | scan (маски) vs полный проход (батч и стрим) | ✅ | — | — | — |
//! | `full_markup` | Полная палитра конструкций (вкл. блочные), батч и стрим | ✅ | ✅ | — | ✅ |
//!
//! Правило сравнения: каждый парсер обязан отдать **всё, что нужно рендереру**.
//! Недопарсенный результат в сравнение не берётся — это занижает конкурента,
//! а не показывает преимущество. Поэтому:
//! - ferromark гоняется полностью: блоки → `take_link_refs` → `fixup_list_tight`
//!   → инлайн по текстовым диапазонам, ровно как в его собственном рендерере,
//!   но без самого рендера;
//! - у обоих конкурентов включены все расширения, которые они умеют
//!   (`ferromark_options`, `pulldown_options`), причём так, чтобы они читали
//!   разметку, а не текст: иначе сопоставимых конструкций просто не будет.
//!
//! Стрим-варианты (`*_stream`, `*_stream_ttfs`) живут в тех же группах,
//! что и батч, на том же документе: `*_stream` — полное время парсинга
//! с отдачей спанов по мере готовности, `*_stream_ttfs` — время до
//! первого спана (time-to-first-span).
//!
//! Документы:
//! - `parse_spans` — **эквивалентные** документы, максимально построчно
//!   совпадающие. Всё, что markdown не выражает нативным синтаксисом или
//!   универсальным raw HTML с тем же смыслом, из документов убрано (спойлеры:
//!   нативной формы нет, а `<details>` — всегда многострочный HTML-блок с
//!   закрывающей пустой строкой, из-за чего совпадение строк ломается; вернём
//!   их отдельной группой). Разметка на каждой стороне должна срабатывать
//!   **по-настоящему**, поэтому `==highlight==` (только ferromark) заменён на
//!   `<mark>`, а таблице добавлен разделитель `| --- | --- |`.
//! - `full_markup` — та же идея на полной палитре конструкций. zoll-документы
//!   не меняются.
//! - Единственная неизбежная асимметрия: markdown требует строку-разделитель
//!   для таблицы, а zoll — нет, поэтому markdown-документы длиннее. Основная
//!   метрика — абсолютное время; в README рядом с ним идёт нс на байт своего
//!   входа.
//!
//! sparkdown (0.1.0) — HTML-only (scaffold, только абзацы), поэтому только
//! в `html_render`. ferromark — публичный API полного разбора: `BlockParser`
//! (блоки) + `InlineParser` (инлайн) + `fixup_list_tight`; HTML отдельной
//! функцией, поэтому и в `parse_spans`, и в `html_render`. Оба парсят тот
//! же `md_doc`, что и pulldown-cmark.
//!
//! Запуск:
//!   cargo bench --bench compare
//!
//! Результаты: `target/criterion/report/index.html`

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::time::{Duration, Instant};

use zoll::engine::{Engine, INTERESTING_BYTES, SpanSink, dispatch_spans, scan};

// ─── 0. Конфигурации конкурентов и полный разбор ferromark ─────
//
// У обоих конкурентов включается всё, что они умеют: сравнивать парсер,
// которому часть грамматики выключили, бессмысленно — он читает документ как
// текст и выигрывает нечестно. Дефолты не годятся: `ferromark::Options::commonmark()`
// и `pulldown_cmark::Parser::new` = `Options::empty()` выключают tables,
// strikethrough, highlight и math, а в тестовом документе есть все четыре.
//
// `render_policy: Trusted` + `disallowed_raw_html: false` — `<u>`, `<ins>`,
// `<del>`, `<sup>`, `<sub>`, `<mark>` в документе это осмысленная разметка,
// и pulldown-cmark их тоже не фильтрует. Untrusted экранировал бы их, а
// GFM-фильтр запрещённого raw HTML отсёк бы часть тегов: обе стороны должны
// читать один и тот же смысл.
//
// `front_matter: false` оставлен намеренно: он не добавляет работы, зато на
// будущем документе, начинающемся с `---`, молча вырезал бы содержимое.
fn ferromark_options() -> ferromark::Options {
    ferromark::Options {
        render_policy: ferromark::RenderPolicy::Trusted,
        disallowed_raw_html: false,
        front_matter: false,
        highlight: true,
        math: true,
        superscript: true,
        subscript: true,
        callouts: true,
        definition_lists: true,
        footnotes: true,
        inline_footnotes: true,
        merged_table_cells: true,
        table_column_widths: true,
        heading_ids: true,
        line_comments: true,
        ..ferromark::Options::gfm()
    }
}

// Все расширения pulldown-cmark: таблицы, сноски, зачёркивание, таски, смарт-
// пунктуация, атрибуты заголовков, метаданные, math, GFM, списки определений,
// верхний/нижний индекс, вики-ссылки. Флаги, для которых в документе нет
// соответствующего синтаксиса, ничего не стоят — зато конкурент точно не
// обвинён в том, что ему что-то недодали.
fn pulldown_options() -> pulldown_cmark::Options {
    pulldown_cmark::Options::all()
}

// Полный путь «файл → структура, готовая к рендеру», без рендера.
// Построчно повторяет `ferromark::render_to_writer_impl` (lib.rs), из
// которого убран только вызов рендера:
//
//   1. `BlockParser::parse` — блок-структура в `Vec<BlockEvent>`;
//   2. `take_link_refs` — ссылки нужно инлайну для резолва;
//   3. `fixup_list_tight` — без него у `ListStart` неизвестен tight/loose
//      (значение приходит из `ListEnd`), т.е. список событий неполон;
//   4. `InlineParser::parse_with_options` по каждому `BlockEvent::Text`
//      — так же, как в рендерере: `Code` и `HtmlBlockText` инлайн не
//      разбираются, а `Text` покрывает абзацы, заголовки и ячейки таблиц.
//
// Буфер инлайн-событий переиспользуется с `clear()`, как в самом рендерере.
// Собирать все события в один плоский список нельзя: ferromark отдаёт их
// по диапазонам, и общий список потребовал бы копирования, которого в
// реальном рендере не происходит. То есть мерится ровно работа парсинга.
//
// Известная асимметрия готовности: `InlineEvent` несёт диапазоны
// относительно среза (рендерер сам прибавляет начало блока), тогда как
// спаны zoll — абсолютные байты документа. Работа одинаковая, структура
// «готовности» разная; на измерение это не влияет.
fn ferromark_full_parse(input: &[u8], opts: &ferromark::Options) {
    let mut parser = ferromark::block::BlockParser::new_with_options(input, opts.clone());
    let mut blocks: Vec<ferromark::block::BlockEvent> =
        Vec::with_capacity((input.len() / 16).max(64));
    parser.parse(&mut blocks);
    let link_refs = parser.take_link_refs();
    ferromark::fixup_list_tight(&mut blocks);

    let mut inline_parser = ferromark::inline::InlineParser::new();
    let mut events: Vec<ferromark::inline::InlineEvent> =
        Vec::with_capacity((input.len() / 8).max(8));
    let refs = opts.allow_link_refs.then_some(&link_refs);
    let mut total = 0usize;

    for event in &blocks {
        if let ferromark::block::BlockEvent::Text(range) = event {
            events.clear();
            inline_parser.parse_with_options(
                range.slice(input),
                refs,
                opts.allow_html,
                opts.strikethrough,
                opts.highlight,
                opts.superscript,
                opts.subscript,
                opts.autolink_literals,
                opts.math,
                false, // inline_footnotes
                None,  // footnote_store
                &mut events,
            );
            total += events.len();
        }
    }

    // black_box по значению, не по длине: иначе компилятор вправе не
    // материализовать содержимое событий.
    black_box((blocks, total));
}

// ─── Генерация тестовых документов ────────────────────────────

const DOC_LINES: usize = 5_000;

// Генерирует zoll-документ (5000 строк, ~390 KB).
// Палитра подобрана так, чтобы каждая конструкция имела эквивалент в
// markdown (нативный или raw HTML с тем же смыслом) — иначе сравнение
// документов перестаёт быть эквивалентным. Спойлеров здесь нет: в markdown
// нативной формы нет, а `<details>` всегда многострочный HTML-блок с
// закрывающей пустой строкой. Вернём их отдельной группой.
fn generate_zoll_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1 Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 11;
        match section {
            0 => s.push_str("#2 Section\n"),
            1 => s.push_str("This is **bold)) and //italic)) text\n"),
            2 => s.push_str("- list item with **bold))\n"),
            3 => s.push_str("1; numbered item with //italic))\n"),
            4 => s.push_str("> quote line with ==highlight))\n"),
            5 => s.push_str("Plain text ~~strike)) __underline))\n"),
            6 => s.push_str("++insert)) --delete)) ''super)) ,,sub))\n"),
            7 => s.push_str("%%this is a comment line}\n"),
            8 => s.push_str("| cell | cell |\n"),
            9 => s.push_str("$$sqrt(x)}\n"),
            10 => s.push_str("plain text line\n"),
            _ => unreachable!(),
        }
    }
    s.push_str("#1 End of Document\n");
    s
}

// Markdown-документ, эквивалентный `generate_zoll_doc` построчно.
//
// Два отличия, и оба вынужденные:
// - `==highlight==` заменён на `<mark>highlight</mark>`: highlight есть
//   только у ferromark, pulldown-cmark не поддерживает его ни одним флагом;
// - таблице добавлена строка-разделитель: GFM требует её, а zoll обходится
//   строкой `| ... |`. Из-за этого markdown-документ длиннее, что учтено
//   метрикой «нс на байт своего входа».
fn generate_md_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("# Benchmark Document\n\n");
    for i in 0..lines.saturating_sub(3) {
        let section = i % 11;
        match section {
            0 => s.push_str("## Section\n"),
            1 => s.push_str("This is **bold** and *italic* text\n"),
            2 => s.push_str("- list item with **bold**\n"),
            3 => s.push_str("1. numbered item with *italic*\n"),
            4 => s.push_str("> quote line with <mark>highlight</mark>\n"),
            5 => s.push_str("Plain text ~~strike~~ <u>underline</u>\n"),
            6 => {
                s.push_str("<ins>insert</ins> <del>delete</del> <sup>super</sup> <sub>sub</sub>\n")
            }
            7 => s.push_str("<!-- this is a comment line -->\n"),
            8 => s.push_str("| cell | cell |\n| --- | --- |\n"),
            9 => s.push_str("$$ sqrt(x) $$\n"),
            10 => s.push_str("plain text line\n"),
            _ => unreachable!(),
        }
    }
    s.push_str("# End of Document\n");
    s
}

// Генерирует zoll-документ с ПОЛНОЙ палитрой конструкций: все inline
// (bold/italic/underline/strike/highlight/insert/delete/super/sub/formula),
// line-level (%%/$$), block-level (%%%/$$$), структура (#N, #:тег, ---,
// списки, цитата, таблица). Спойлеры исключены — см. `generate_zoll_doc`.
// Цикл из 13 секций: 11 однострочных + 2 блочных (по 3 строки) = 17 строк.
fn generate_full_markup_doc(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("#1 Full Markup Benchmark\n\n");
    let cycles = lines * 13 / 17;
    for i in 0..cycles {
        let section = i % 13;
        match section {
            0 => s.push_str("#2 Section\n"),
            1 => s.push_str("**bold)) //italic)) __underline)) ~~strike))\n"),
            2 => s.push_str("==highlight)) ++insert)) --delete)) ''super)) ,,sub)) $x))\n"),
            3 => s.push_str("- list item with **bold))\n"),
            4 => s.push_str("1; numbered item with //italic))\n"),
            5 => s.push_str("> quote line with ==highlight))\n"),
            6 => s.push_str("#:tag\n"),
            7 => s.push_str("---\n"),
            8 => s.push_str("| cell | cell |\n"),
            9 => s.push_str("%%comment}\n"),
            10 => s.push_str("$$sqrt(x)}\n"),
            11 => s.push_str("%%%\nblock comment\n}\n"),
            12 => s.push_str("$$$\nblock formula\n}\n"),
            _ => unreachable!(),
        }
    }
    s.push_str("#1 End of Document\n");
    s
}

// Markdown-эквивалент полной палитры. Три отличия от `generate_full_markup_doc`,
// все вынужденные и все такие, что разметка срабатывает по-настоящему:
// - `==highlight==` → `<mark>highlight</mark>`: highlight есть только у
//   ferromark, pulldown-cmark не поддерживает его ни одним флагом;
// - таблице добавлен разделитель `| --- | --- |`: GFM требует его, zoll — нет;
// - `#:tag` переводится в `<!-- tag -->`: тегов в markdown нет, это ближайшее
//   нативное construct'ное место (комментарий, а не тег).
// Спойлеров нет по причине, описанной в `generate_zoll_doc`.
// Цикл из 13 секций: 11 однострочных + 2 блочных = 17 строк (плюс строка
// разделителя таблицы, поэтому markdown-документ длиннее zoll-документа).
fn generate_full_markup_md(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 80);
    s.push_str("# Full Markup Benchmark\n\n");
    let cycles = lines * 13 / 17;
    for i in 0..cycles {
        let section = i % 13;
        match section {
            0 => s.push_str("## Section\n"),
            1 => s.push_str("**bold** *italic* <u>underline</u> ~~strike~~\n"),
            2 => s.push_str(
                "<mark>highlight</mark> <ins>insert</ins> <del>delete</del> \
                 <sup>super</sup> <sub>sub</sub> $x$\n",
            ),
            3 => s.push_str("- list item with **bold**\n"),
            4 => s.push_str("1. numbered item with *italic*\n"),
            5 => s.push_str("> quote line with <mark>highlight</mark>\n"),
            6 => s.push_str("<!-- tag -->\n"),
            7 => s.push_str("---\n"),
            8 => s.push_str("| cell | cell |\n| --- | --- |\n"),
            9 => s.push_str("<!-- comment -->\n"),
            10 => s.push_str("$$ x = y $$\n"),
            11 => s.push_str("<!--\nblock comment\n-->\n"),
            12 => s.push_str("$$\nblock formula\n$$\n"),
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
    // Батч: парсинг целиком, потом рассылка всех спанов разом — доставка
    // включена, как и у стрима (иначе сравнение нечестное).
    group.bench_function("zoll_engine_parse", |b| {
        b.iter(|| {
            let engine = Engine::parse(black_box(zoll_doc.as_bytes()));
            let mut sink = BenchSink::new();
            dispatch_spans(&mut sink, engine.revision, engine.spans());
            black_box(sink.count);
        });
    });

    // Стрим на том же документе: спаны уходят по мере готовности.
    group.bench_function("zoll_engine_parse_stream", |b| {
        b.iter(|| {
            let mut sink = BenchSink::new();
            let engine = Engine::parse_into(black_box(zoll_doc.as_bytes()), &mut sink);
            black_box((engine.spans().len(), sink.count));
        });
    });

    // Время до первого спана (TTFS): редактор начинает рисовать раньше.
    group.bench_function("zoll_engine_parse_stream_ttfs", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut sink = BenchSink::new();
                Engine::parse_into(black_box(zoll_doc.as_bytes()), &mut sink);
                total += sink.first_span_at.unwrap().duration_since(start);
            }
            total
        });
    });

    group.throughput(Throughput::Bytes(md_doc.len() as u64));
    // Все расширения включены: иначе зачёркивание, таблицы и формулы идут
    // для pulldown обычным текстом (см. `pulldown_options`).
    let pulldown_opts = pulldown_options();
    group.bench_function("pulldown_cmark_events", |b| {
        b.iter(|| {
            let events: Vec<pulldown_cmark::Event> =
                pulldown_cmark::Parser::new_ext(black_box(&md_doc), pulldown_opts).collect();
            black_box(events);
        });
    });

    // ferromark: полный разбор — блоки + fixup + инлайн (см.
    // ferromark_full_parse), результат готов к рендеру.
    let ferromark_opts = ferromark_options();
    group.bench_function("ferromark_full_parse", |b| {
        b.iter(|| {
            ferromark_full_parse(black_box(md_doc.as_bytes()), &ferromark_opts);
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

    let pulldown_opts = pulldown_options();
    group.bench_function("pulldown_cmark_html", |b| {
        b.iter(|| {
            let parser = pulldown_cmark::Parser::new_ext(black_box(&md_doc), pulldown_opts);
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

    // ferromark: та же конфигурация, что и в parse_spans (все расширения,
    // Trusted без GFM-фильтра raw HTML), чтобы обе группы мерили одну и ту
    // же грамматику.
    let ferromark_opts = ferromark_options();
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

    // Полный парсинг со стрим-отдачей спанов по мере готовности.
    group.bench_function("engine_parse_stream", |b| {
        b.iter(|| {
            let mut sink = BenchSink::new();
            let engine = Engine::parse_into(black_box(text), &mut sink);
            black_box((engine.spans().len(), sink.count));
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
    // Батч: парсинг целиком, потом рассылка всех спанов разом — доставка
    // включена, как и у стрима (иначе сравнение нечестное).
    group.bench_function("zoll_engine_parse", |b| {
        b.iter(|| {
            let engine = Engine::parse(black_box(zoll_doc.as_bytes()));
            let mut sink = BenchSink::new();
            dispatch_spans(&mut sink, engine.revision, engine.spans());
            black_box(sink.count);
        });
    });

    // Стрим на том же документе: спаны уходят по мере готовности.
    group.bench_function("zoll_engine_parse_stream", |b| {
        b.iter(|| {
            let mut sink = BenchSink::new();
            let engine = Engine::parse_into(black_box(zoll_doc.as_bytes()), &mut sink);
            black_box((engine.spans().len(), sink.count));
        });
    });

    // Время до первого спана (TTFS): редактор начинает рисовать раньше.
    group.bench_function("zoll_engine_parse_stream_ttfs", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut sink = BenchSink::new();
                Engine::parse_into(black_box(zoll_doc.as_bytes()), &mut sink);
                total += sink.first_span_at.unwrap().duration_since(start);
            }
            total
        });
    });

    group.throughput(Throughput::Bytes(md_doc.len() as u64));
    let pulldown_opts = pulldown_options();
    group.bench_function("pulldown_cmark_events", |b| {
        b.iter(|| {
            let events: Vec<pulldown_cmark::Event> =
                pulldown_cmark::Parser::new_ext(black_box(&md_doc), pulldown_opts).collect();
            black_box(events);
        });
    });

    let ferromark_opts = ferromark_options();
    group.bench_function("ferromark_full_parse", |b| {
        b.iter(|| {
            ferromark_full_parse(black_box(md_doc.as_bytes()), &ferromark_opts);
        });
    });

    group.finish();
}

// ─── 5. Стрим-доставка спанов ──────────────────────────────────
//
// Стрим отдаёт спаны по мере готовности, батч — все разом после
// парсинга. Стрим-варианты живут в тех же группах, что и батч, на
// том же документе — сравнение честное. Метрики:
// - *_stream: полное время (парсинг + доставка);
// - *_stream_ttfs: время до ПЕРВОГО спана (time-to-first-span) —
//   главный выигрыш стрима: редактор начинает рисовать раньше.

// Синк для бенча: считает спаны и запоминает время первого вызова.
struct BenchSink {
    count: usize,
    first_span_at: Option<Instant>,
}

impl BenchSink {
    fn new() -> Self {
        BenchSink {
            count: 0,
            first_span_at: None,
        }
    }

    fn record(&mut self) {
        if self.count == 0 {
            self.first_span_at = Some(Instant::now());
        }
        self.count += 1;
    }
}

// Все ручки одинаковые: посчитать спан. Макрос вместо 30 копипаст.
macro_rules! bench_sink_methods {
    ($($method:ident),* $(,)?) => {
        $(
            fn $method(&mut self, _start: usize, _end: usize) {
                self.record();
            }
        )*
    };
}

impl SpanSink for BenchSink {
    fn begin_revision(&mut self, _revision: u64) {}
    bench_sink_methods!(
        on_bold,
        on_italic,
        on_underline,
        on_strikethrough,
        on_highlight,
        on_insertion,
        on_deletion,
        on_superscript,
        on_subscript,
        on_formula_inline,
        on_comment_inline,
        on_spoiler_inline,
        on_code_inline,
        on_tag,
        on_quote,
        on_list_item,
        on_table_row,
        on_thematic_break,
        on_formula_line,
        on_comment_line,
        on_spoiler_line,
        on_code_line,
        on_formula_block,
        on_comment_block,
        on_spoiler_block,
        on_code_block,
        on_metadata,
    );
    fn on_header(&mut self, _start: usize, _end: usize, _level: u8) {
        self.record();
    }
    fn end_revision(&mut self) {}
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
