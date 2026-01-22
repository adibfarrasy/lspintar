use tower_lsp::lsp_types::Position;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::language_support::ParameterResult;

pub fn get_one(node: &Node, content: &str, query: &Query) -> Option<String> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, *node, content.as_bytes());

    matches.next().and_then(|m| {
        m.captures
            .first()
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok().map(String::from))
    })
}

pub fn get_many(node: &Node, content: &str, query: &Query, max_depth: Option<u32>) -> Vec<String> {
    let mut cursor = QueryCursor::new();
    if let Some(depth) = max_depth {
        cursor.set_max_start_depth(Some(depth));
    }

    let mut results = Vec::new();
    cursor
        .matches(&query, *node, content.as_bytes())
        .for_each(|m| {
            for capture in m.captures {
                if let Ok(text) = capture.node.utf8_text(content.as_bytes()) {
                    results.push(text.to_string());
                }
            }
        });
    results
}

pub fn parse_parameter(param: &str) -> ParameterResult {
    let param = param.trim();
    param
        .split_once(':')
        .map(|(name, rest)| {
            if let Some((arg, default)) = rest.split_once('=') {
                (
                    name.trim().to_string(),
                    Some(arg.trim().to_string()),
                    Some(default.trim().to_string()),
                )
            } else {
                (name.trim().to_string(), Some(rest.trim().to_string()), None)
            }
        })
        .unwrap_or((param.to_string(), None, None))
}

pub fn node_contains_position(node: &Node, position: &Position) -> bool {
    let (start, end) = (node.start_position(), node.end_position());
    let (line, char) = (position.line as usize, position.character as usize);

    (start.row < line || (start.row == line && start.column <= char))
        && (line < end.row || (line == end.row && char <= end.column))
}

pub fn get_node_at_position<'a>(
    tree: &'a Tree,
    content: &str,
    position: &Position,
) -> Option<Node<'a>> {
    let mut line = 0;
    let mut col = 0;
    let mut byte_offset = 0;

    for ch in content.chars() {
        if line == position.line as usize && col == position.character as usize {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }

        byte_offset += ch.len_utf8();
    }

    tree.root_node()
        .descendant_for_byte_range(byte_offset, byte_offset)
}
