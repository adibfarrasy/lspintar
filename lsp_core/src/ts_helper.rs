use tower_lsp::lsp_types::Position;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::language_support::ParameterResult;

pub fn get_one(node: &Node, content: &str, query: &Query) -> Option<String> {
    get_one_with_position(node, content, query).map(|(text, _)| text)
}

pub fn get_one_with_position(
    node: &Node,
    content: &str,
    query: &Query,
) -> Option<(String, Position)> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, *node, content.as_bytes());
    matches.next().and_then(|m| {
        m.captures.first().and_then(|c| {
            let text = c
                .node
                .utf8_text(content.as_bytes())
                .ok()
                .map(String::from)?;
            let position = Position {
                line: c.node.start_position().row as u32,
                character: c.node.start_position().column as u32,
            };
            Some((text, position))
        })
    })
}

pub fn get_many_with_position(
    node: &Node,
    content: &str,
    query: &Query,
    max_depth: Option<u32>,
) -> Vec<(String, Position)> {
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
                    let position = Position {
                        line: capture.node.start_position().row as u32,
                        character: capture.node.start_position().column as u32,
                    };
                    results.push((text.to_string(), position));
                }
            }
        });
    results
}

pub fn get_many(node: &Node, content: &str, query: &Query, max_depth: Option<u32>) -> Vec<String> {
    get_many_with_position(node, content, query, max_depth)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

pub fn parse_parameter(param: &str) -> ParameterResult {
    let param = param.trim();

    let (type_and_name, default) = if let Some(eq_pos) = param.find('=') {
        let (left, right) = param.split_at(eq_pos);
        (left.trim(), Some(right[1..].trim().trim_matches('\'')))
    } else {
        (param, None)
    };

    if let Some(last_space) = type_and_name.rfind(char::is_whitespace) {
        let type_name = type_and_name[..last_space].trim().to_string();
        let name = type_and_name[last_space..].trim().to_string();
        (name, Some(type_name), default.map(String::from))
    } else {
        // No whitespace = untyped parameter (for Groovy)
        (type_and_name.to_string(), None, default.map(String::from))
    }
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
