//! Merge-функция: собирает строки (LineAST) в общий AST (MarkupDoc).

use crate::ast::{BlockType, LineAST, MarkupDoc, MarkupNode};

/// Собирает `Vec<LineAST>` (по одному на строку) в `MarkupDoc`.
pub fn merge(lines: &[LineAST]) -> MarkupDoc {
    let mut doc = MarkupDoc {
        children: Vec::new(),
    };

    // Стек блок-левел маркеров
    let mut block_stack: Vec<BlockFrame> = Vec::new();

    // Временные буферы для группировки
    let mut para_buffer: Vec<Vec<MarkupNode>> = Vec::new();
    let mut quote_buffer: Vec<Vec<MarkupNode>> = Vec::new();
    let mut list_items: Vec<ListItemData> = Vec::new();
    let mut list_ordered: Option<bool> = None;
    let mut table_rows: Vec<TableRowData> = Vec::new();

    let mut flush_paragraph = |doc: &mut MarkupDoc, buffer: &mut Vec<Vec<MarkupNode>>| {
        if buffer.is_empty() {
            return;
        }
        if buffer.len() == 1 {
            doc.children.append(&mut buffer.swap_remove(0));
        } else {
            let mut all = buffer.swap_remove(0);
            for next_line in buffer.drain(..) {
                all.push(MarkupNode::Text("\n".to_string()));
                all.extend(next_line);
            }
            doc.children.append(&mut all);
        }
    };

    let mut flush_quote = |doc: &mut MarkupDoc, buffer: &mut Vec<Vec<MarkupNode>>| {
        if buffer.is_empty() {
            return;
        }
        let mut all = buffer.swap_remove(0);
        for next_line in buffer.drain(..) {
            all.push(MarkupNode::Text("\n".to_string()));
            all.extend(next_line);
        }
        doc.children.push(MarkupNode::Quote(all));
    };

    let mut flush_list = |doc: &mut MarkupDoc,
                          items: &mut Vec<ListItemData>,
                          ordered: &mut Option<bool>| {
        if items.is_empty() {
            return;
        }
        for item in items.drain(..) {
            doc.children.push(MarkupNode::ListItem {
                ordered: item.ordered,
                number: item.number,
                children: item.children,
            });
        }
        *ordered = None;
    };

    let mut flush_table = |doc: &mut MarkupDoc, rows: &mut Vec<TableRowData>| {
        if rows.is_empty() {
            return;
        }
        for row in rows.drain(..) {
            doc.children.push(MarkupNode::TableRow(row.cells));
        }
    };

    for line_ast in lines {
        match line_ast {
            LineAST::BlockMarker(bt) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );

                if let Some(top) = block_stack.last() {
                    if top.block_type == *bt {
                        let frame = block_stack.pop().unwrap();
                        let node = block_to_node(*bt, frame.title, frame.content);
                        if let Some(parent) = block_stack.last_mut() {
                            parent.content.push(node);
                        } else {
                            doc.children.push(node);
                        }
                        continue;
                    }
                }
                block_stack.push(BlockFrame {
                    block_type: *bt,
                    title: None,
                    content: Vec::new(),
                });
            }

            LineAST::SpoilerBlockOpen(title) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );

                block_stack.push(BlockFrame {
                    block_type: BlockType::Spoiler,
                    title: title.clone(),
                    content: Vec::new(),
                });
            }

            LineAST::CodeLine(text) => {
                if let Some(top) = block_stack.last_mut() {
                    top.add_line(text.clone());
                }
            }
            LineAST::CommentLine(text) => {
                if let Some(top) = block_stack.last_mut() {
                    top.add_line(text.clone());
                }
            }
            LineAST::FormulaLine(text) => {
                if let Some(top) = block_stack.last_mut() {
                    top.add_line(text.clone());
                }
            }
            LineAST::SpoilerLine(children) => {
                if let Some(top) = block_stack.last_mut() {
                    top.content.extend(children.clone());
                    top.content.push(MarkupNode::Text("\n".to_string()));
                }
            }

            LineAST::Paragraph(children) => {
                if !block_stack.is_empty() {
                    block_stack.last_mut().unwrap().content.extend(children.clone());
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Text("\n".to_string()));
                    continue;
                }
                flush_quote(&mut doc, &mut quote_buffer);
                flush_list(&mut doc, &mut list_items, &mut list_ordered);
                flush_table(&mut doc, &mut table_rows);
                if children.is_empty() {
                    continue;
                }
                para_buffer.push(children.clone());
            }

            LineAST::Quote(children) => {
                if !block_stack.is_empty() {
                    block_stack.last_mut().unwrap().content.extend(children.clone());
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Text("\n".to_string()));
                    continue;
                }
                flush_paragraph(&mut doc, &mut para_buffer);
                flush_list(&mut doc, &mut list_items, &mut list_ordered);
                flush_table(&mut doc, &mut table_rows);
                quote_buffer.push(children.clone());
            }

            LineAST::ListItem(ordered, number, children) => {
                if !block_stack.is_empty() {
                    block_stack.last_mut().unwrap().content.extend(children.clone());
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Text("\n".to_string()));
                    continue;
                }
                flush_paragraph(&mut doc, &mut para_buffer);
                flush_quote(&mut doc, &mut quote_buffer);
                flush_table(&mut doc, &mut table_rows);
                if let Some(is_ordered) = list_ordered {
                    if is_ordered != *ordered {
                        flush_list(&mut doc, &mut list_items, &mut list_ordered);
                    }
                }
                list_ordered = Some(*ordered);
                list_items.push(ListItemData {
                    ordered: *ordered,
                    number: *number,
                    children: children.clone(),
                });
            }

            LineAST::TableRow(cells) => {
                if !block_stack.is_empty() {
                    block_stack.last_mut().unwrap().content.push(MarkupNode::TableRow(cells.clone()));
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Text("\n".to_string()));
                    continue;
                }
                flush_paragraph(&mut doc, &mut para_buffer);
                flush_quote(&mut doc, &mut quote_buffer);
                flush_list(&mut doc, &mut list_items, &mut list_ordered);
                table_rows.push(TableRowData {
                    cells: cells.clone(),
                });
            }

            LineAST::Header(level, children) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
                if !block_stack.is_empty() {
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Header {
                        level: *level,
                        children: children.clone(),
                    });
                } else {
                    doc.children.push(MarkupNode::Header {
                        level: *level,
                        children: children.clone(),
                    });
                }
            }

            LineAST::Spoiler(title, children) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
                let node = if block_stack.is_empty() {
                    &mut doc.children
                } else {
                    &mut block_stack.last_mut().unwrap().content
                };
                node.push(MarkupNode::Spoiler {
                    title: title.clone(),
                    children: children.clone(),
                });
            }

            LineAST::Comment(children) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
                if block_stack.is_empty() {
                    doc.children.push(MarkupNode::Comment(children.clone()));
                } else {
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Comment(children.clone()));
                }
            }

            LineAST::Formula(children) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
                if block_stack.is_empty() {
                    doc.children.push(MarkupNode::Formula(children.clone()));
                } else {
                    block_stack.last_mut().unwrap().content.push(MarkupNode::Formula(children.clone()));
                }
            }

            LineAST::ThematicBreak => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
                if block_stack.is_empty() {
                    doc.children.push(MarkupNode::ThematicBreak);
                } else {
                    block_stack.last_mut().unwrap().content.push(MarkupNode::ThematicBreak);
                }
            }

            LineAST::Tag(_tag) => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
            }

            LineAST::Empty => {
                flush_all(
                    &mut doc,
                    &mut para_buffer,
                    &mut quote_buffer,
                    &mut list_items,
                    &mut list_ordered,
                    &mut table_rows,
                    &mut flush_paragraph,
                    &mut flush_quote,
                    &mut flush_list,
                    &mut flush_table,
                );
            }
        }
    }

    flush_paragraph(&mut doc, &mut para_buffer);
    flush_quote(&mut doc, &mut quote_buffer);
    flush_list(&mut doc, &mut list_items, &mut list_ordered);
    flush_table(&mut doc, &mut table_rows);

    while let Some(frame) = block_stack.pop() {
        let node = block_to_node(frame.block_type, frame.title, frame.content);
        if let Some(parent) = block_stack.last_mut() {
            parent.content.push(node);
        } else {
            doc.children.push(node);
        }
    }

    doc
}

// ─── Внутренние типы ──────────────────────────────────────────

struct BlockFrame {
    block_type: BlockType,
    title: Option<String>,
    content: Vec<MarkupNode>,
}

impl BlockFrame {
    fn add_line(&mut self, text: String) {
        if !self.content.is_empty() {
            self.content.push(MarkupNode::Text("\n".to_string()));
        }
        self.content.push(MarkupNode::Text(text));
    }
}

struct ListItemData {
    ordered: bool,
    number: u32,
    children: Vec<MarkupNode>,
}

struct TableRowData {
    cells: Vec<Vec<MarkupNode>>,
}

// ─── Помощники ────────────────────────────────────────────────

fn block_to_node(bt: BlockType, title: Option<String>, content: Vec<MarkupNode>) -> MarkupNode {
    match bt {
        BlockType::Comment => MarkupNode::Comment(content),
        BlockType::Formula => MarkupNode::Formula(content),
        BlockType::Spoiler => MarkupNode::Spoiler { title, children: content },
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_all(
    doc: &mut MarkupDoc,
    para_buffer: &mut Vec<Vec<MarkupNode>>,
    quote_buffer: &mut Vec<Vec<MarkupNode>>,
    list_items: &mut Vec<ListItemData>,
    list_ordered: &mut Option<bool>,
    table_rows: &mut Vec<TableRowData>,
    flush_paragraph: &mut impl FnMut(&mut MarkupDoc, &mut Vec<Vec<MarkupNode>>),
    flush_quote: &mut impl FnMut(&mut MarkupDoc, &mut Vec<Vec<MarkupNode>>),
    flush_list: &mut impl FnMut(&mut MarkupDoc, &mut Vec<ListItemData>, &mut Option<bool>),
    flush_table: &mut impl FnMut(&mut MarkupDoc, &mut Vec<TableRowData>),
) {
    flush_paragraph(doc, para_buffer);
    flush_quote(doc, quote_buffer);
    flush_list(doc, list_items, list_ordered);
    flush_table(doc, table_rows);
}

// ─── Полный парсинг (удобство) ────────────────────────────────

/// Парсит текст целиком: разбивка на строки → parse_line → merge.
pub fn parse_full(text: &str) -> MarkupDoc {
    let lines: Vec<LineAST> = text.lines().map(|l| crate::parser::parse_line(l)).collect();
    merge(&lines)
}

// ─── Тесты ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::MarkupNode;

    #[test]
    fn plain_text_paragraph() {
        let lines = vec![
            LineAST::Paragraph(vec![MarkupNode::Text("hello".to_string())]),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 1);
        assert_eq!(doc.children[0], MarkupNode::Text("hello".to_string()));
    }

    #[test]
    fn multi_line_paragraph() {
        let lines = vec![
            LineAST::Paragraph(vec![MarkupNode::Text("line 1".to_string())]),
            LineAST::Paragraph(vec![MarkupNode::Text("line 2".to_string())]),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 3);
        assert_eq!(doc.children[0], MarkupNode::Text("line 1".to_string()));
        assert_eq!(doc.children[1], MarkupNode::Text("\n".to_string()));
        assert_eq!(doc.children[2], MarkupNode::Text("line 2".to_string()));
    }

    #[test]
    fn header_in_doc() {
        let lines = vec![
            LineAST::Header(1, vec![MarkupNode::Text("Title".to_string())]),
            LineAST::Paragraph(vec![MarkupNode::Text("content".to_string())]),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 2);
        assert_eq!(doc.children[0], MarkupNode::Header {
            level: 1,
            children: vec![MarkupNode::Text("Title".to_string())],
        });
    }

    #[test]
    fn block_comment() {
        let lines = vec![
            LineAST::BlockMarker(BlockType::Comment),
            LineAST::Paragraph(vec![MarkupNode::Text("hidden".to_string())]),
            LineAST::BlockMarker(BlockType::Comment),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 1);
        assert_eq!(doc.children[0], MarkupNode::Comment(vec![
            MarkupNode::Text("hidden".to_string()),
            MarkupNode::Text("\n".to_string()),
        ]));
    }

    #[test]
    fn block_spoiler_with_title() {
        let lines = vec![
            LineAST::SpoilerBlockOpen(Some("spoil".to_string())),
            LineAST::Paragraph(vec![MarkupNode::Text("content".to_string())]),
            LineAST::BlockMarker(BlockType::Spoiler),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 1);
        assert_eq!(doc.children[0], MarkupNode::Spoiler {
            title: Some("spoil".to_string()),
            children: vec![
                MarkupNode::Text("content".to_string()),
                MarkupNode::Text("\n".to_string()),
            ],
        });
    }

    #[test]
    fn list_grouping() {
        let lines = vec![
            LineAST::ListItem(false, 0, vec![MarkupNode::Text("one".to_string())]),
            LineAST::ListItem(false, 0, vec![MarkupNode::Text("two".to_string())]),
            LineAST::ListItem(false, 0, vec![MarkupNode::Text("three".to_string())]),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 3);
        for item in &doc.children {
            assert!(matches!(item, MarkupNode::ListItem { ordered: false, .. }));
        }
    }

    #[test]
    fn empty_lines_separate_paragraphs() {
        let lines = vec![
            LineAST::Paragraph(vec![MarkupNode::Text("p1".to_string())]),
            LineAST::Empty,
            LineAST::Paragraph(vec![MarkupNode::Text("p2".to_string())]),
        ];
        let doc = merge(&lines);
        assert_eq!(doc.children.len(), 2);
        assert_eq!(doc.children[0], MarkupNode::Text("p1".to_string()));
        assert_eq!(doc.children[1], MarkupNode::Text("p2".to_string()));
    }
}
