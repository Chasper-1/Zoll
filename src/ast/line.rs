use super::node::MarkupNode;

/// Тип блок-левел маркера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Comment,
    Formula,
    Spoiler,
}

/// AST одной строки.
///
/// Каждая строка документа парсится независимо в `LineAST`.
/// Merge-функция собирает из них общий `MarkupDoc`.
#[derive(Debug, Clone, PartialEq)]
pub enum LineAST {
    /// Пустая строка (только пробелы/перенос).
    Empty,

    // ─── Inline-строка ────────────────────────────────────────────
    /// Обычный текст с inline-маркерами.
    Paragraph(Vec<MarkupNode>),

    // ─── Line-level маркеры ───────────────────────────────────────
    /// %% комментарий до конца строки
    Comment(Vec<MarkupNode>),
    /// $$ формула до конца строки
    Formula(Vec<MarkupNode>),
    /// !! спойлер до конца строки (с опциональным заголовком)
    Spoiler(Option<String>, Vec<MarkupNode>),

    // ─── Структурные line-level маркеры ───────────────────────────
    /// #N# Заголовок
    Header(u32, Vec<MarkupNode>),
    /// Элемент списка (ordered, number, дети)
    ListItem(bool, u32, Vec<MarkupNode>),
    /// > Цитата
    Quote(Vec<MarkupNode>),
    /// Строка таблицы: | ячейка 1 | ячейка 2 |
    TableRow(Vec<Vec<MarkupNode>>),
    /// Тэг: #:tag_name
    Tag(String),
    /// Горизонтальный разделитель
    ThematicBreak,

    // ─── Block-level маркеры ──────────────────────────────────────
    /// `%%%` / `$$$` / `!!!` — merge решает open или close.
    BlockMarker(BlockType),
    /// `!!!title:` — точно open (с заголовком).
    SpoilerBlockOpen(Option<String>),

    // ─── Внутри блоков ────────────────────────────────────────────
    /// Строка внутри блока кода (не парсится, просто текст).
    CodeLine(String),
    /// Строка внутри блочного комментария (не парсится).
    CommentLine(String),
    /// Строка внутри блочной формулы (не парсится).
    FormulaLine(String),
    /// Строка внутри блочного спойлера (парсится как обычная).
    SpoilerLine(Vec<MarkupNode>),
}
