use super::node::MarkupNode;

/// Документ — корень AST.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkupDoc {
    pub children: Vec<MarkupNode>,
}
