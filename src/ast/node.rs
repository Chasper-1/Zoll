use super::style::MarkupStyle;

/// Узел AST.
///
/// Больше не содержит `Span` — позиции теперь привязаны к строкам,
/// а не к абсолютным байтам.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkupNode {
    /// Простой текст.
    Text(String),
    /// Форматированный блок с вложенными узлами (inline-маркеры).
    Formatted {
        style: MarkupStyle,
        children: Vec<MarkupNode>,
    },

    // ─── Блок-левел (появляются после merge) ──────────────────────

    /// Заголовок: #1# Title
    Header {
        level: u32,
        children: Vec<MarkupNode>,
    },

    /// Элемент списка (нумерованного или нет).
    ListItem {
        ordered: bool,
        number: u32, // 0 для маркированного, n для нумерованного
        children: Vec<MarkupNode>,
    },

    /// Цитата > текст
    Quote(Vec<MarkupNode>),

    /// Блок кода ``` ... ``` (язык может быть пустым).
    CodeBlock {
        language: String,
        content: String,
    },

    /// Горизонтальный разделитель.
    ThematicBreak,

    /// Строка таблицы (каждая ячейка — inline-контент).
    TableRow(Vec<Vec<MarkupNode>>),

    /// Блочный спойлер (содержимое скрыто до взаимодействия).
    Spoiler {
        title: Option<String>,
        children: Vec<MarkupNode>,
    },

    /// Блочный комментарий (содержимое не отображается).
    Comment(Vec<MarkupNode>),

    /// Блочная формула (LaTeX на нескольких строках).
    Formula(Vec<MarkupNode>),
}
