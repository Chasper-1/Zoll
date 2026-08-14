//! Система зависимостей (раздел 18 спеки).
//!
//! Простая версия: спаны, отсортированные по позиции, с быстрым запросом
//! «какие спаны лежат в диапазоне». При правке перерешаются только
//! затронутые диапазоны — неизменённые области не анализируются повторно.

use crate::engine::resolver::SyntaxSpan;

/// Граф зависимостей спанов.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Спаны, отсортированные по `(start, end)`.
    spans: Vec<SyntaxSpan>,
}

impl DependencyGraph {
    pub fn new(spans: &[SyntaxSpan]) -> Self {
        let mut spans = spans.to_vec();
        spans.sort_by_key(|s| (s.start, s.end));
        DependencyGraph { spans }
    }

    /// Спаны, полностью лежащие в `[start, end)`.
    pub fn spans_in(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.spans
            .iter()
            .filter(|s| s.start >= start && s.end <= end)
            .collect()
    }

    /// Спаны, пересекающие `[start, end)`.
    pub fn spans_overlapping(&self, start: usize, end: usize) -> Vec<&SyntaxSpan> {
        self.spans
            .iter()
            .filter(|s| s.start < end && s.end > start)
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
        let graph = DependencyGraph::new(&[span(0, 5), span(3, 8), span(10, 15)]);
        let in_range = graph.spans_in(2, 9);
        assert_eq!(in_range.len(), 1);
        assert_eq!(in_range[0].start, 3);
    }

    #[test]
    fn spans_overlapping() {
        let graph = DependencyGraph::new(&[span(0, 5), span(3, 8), span(10, 15)]);
        let overlapping = graph.spans_overlapping(4, 11);
        // (0,5): 0<11 && 5>4 ✓; (3,8): 3<11 && 8>4 ✓; (10,15): 10<11 && 15>4 ✓
        assert_eq!(overlapping.len(), 3);
    }

    #[test]
    fn sorted_by_start() {
        let graph = DependencyGraph::new(&[span(10, 15), span(0, 5), span(3, 8)]);
        let starts: Vec<usize> = graph.spans.iter().map(|s| s.start).collect();
        assert_eq!(starts, vec![0, 3, 10]);
    }
}
