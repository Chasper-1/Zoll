//! Система зависимостей (раздел 18 спеки).
//!
//! Простая версия: спаны в порядке построения (отсортированы по позиции,
//! кроме блочных, которые закрываются позже своего открытия), с линейным
//! запросом «какие спаны лежат в диапазоне». При правке пересобирается
//! весь документ; оптимизация «только затронутые диапазоны» — следующий
//! шаг.

use crate::engine::resolver::SyntaxSpan;

// Граф зависимостей спанов.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    // Спаны в порядке построения (единственная копия в движке).
    spans: Vec<SyntaxSpan>,
}

impl DependencyGraph {
    // Принимает владение спанами: копия не нужна, движок хранит одну
    // копию. Сортировка не требуется — запросы линейные.
    pub fn new(spans: Vec<SyntaxSpan>) -> Self {
        DependencyGraph { spans }
    }

    // Спаны в порядке построения.
    pub fn spans(&self) -> &[SyntaxSpan] {
        &self.spans
    }

    // Спаны, полностью лежащие в `[start, end)`.
    pub fn spans_in(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.spans
            .iter()
            .filter(|span| span.start >= start && span.end <= end)
            .collect()
    }

    // Спаны, пересекающие `[start, end)`.
    pub fn spans_overlapping(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.spans
            .iter()
            .filter(|span| span.start < end && span.end > start)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::resolver::SyntaxKind;

    fn span(start: usize, end: usize) -> SyntaxSpan {
        SyntaxSpan {
            start,
            end,
            kind: SyntaxKind::Bold,
        }
    }

    #[test]
    fn spans_in_range() {
        let graph = DependencyGraph::new(vec![span(0, 5), span(3, 8), span(10, 15)]);
        let in_range = graph.spans_in(2, 9);
        assert_eq!(in_range.len(), 1);
        assert_eq!(in_range[0].start, 3);
    }

    #[test]
    fn spans_overlapping() {
        let graph = DependencyGraph::new(vec![span(0, 5), span(3, 8), span(10, 15)]);
        let overlapping = graph.spans_overlapping(4, 11);
        // (0,5): 0<11 && 5>4 ✓; (3,8): 3<11 && 8>4 ✓; (10,15): 10<11 && 15>4 ✓
        assert_eq!(overlapping.len(), 3);
    }

    #[test]
    fn keeps_construction_order() {
        // Спаны хранятся в порядке построения, без сортировки.
        let graph = DependencyGraph::new(vec![span(10, 15), span(0, 5), span(3, 8)]);
        let starts: Vec<usize> = graph.spans().iter().map(|span| span.start).collect();
        assert_eq!(starts, vec![10, 0, 3]);
    }
}
