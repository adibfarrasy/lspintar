use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

pub fn find_identifier_at_position<'a>(
    tree: &'a Tree,
    source: &str,
    position: Position,
) -> Result<Node<'a>> {
    let query_text = r#"
    (identifier) @identifier
    (type_identifier) @identifier
    "#;
    let query = Query::new(&tree.language(), query_text).context(format!(
        "[find_identifier_at_position] failed to create a new query"
    ))?;

    let mut result: Result<Node> = Err(anyhow!(format!(
        "[find_identifier_at_position] invalid data. position: {:#?}",
        position
    )));
    let mut found = false;

    let mut cursor = QueryCursor::new();
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|match_| {
            if found {
                return;
            };

            for capture in match_.captures.iter() {
                let node = capture.node;
                if node_contains_position(&node, position) {
                    result = Ok(node);
                    found = true;
                    return;
                }
            }
        });

    result
}

fn node_contains_position(node: &Node, position: Position) -> bool {
    let start = node.start_position();
    let end = node.end_position();

    let pos_line = position.line as usize;
    let pos_char = position.character as usize;

    (start.row < pos_line || (start.row == pos_line && start.column <= pos_char))
        && (pos_line < end.row || (pos_line == end.row && pos_char <= end.column))
}
