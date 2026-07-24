use crate::ast::style::MarkupStyle;

/// Определение маркера разметки.
#[derive(Debug, Clone)]
pub struct MarkerDef {
    pub open: &'static str,
    pub close: &'static str,
    pub style: MarkupStyle,
    pub multiline: bool,
    /// Отслеживать вложенность одноимённых маркеров.
    pub track_depth: bool,
}
