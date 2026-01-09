use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

use crate::language_support::ParameterResult;

pub fn get_one(
    language: tree_sitter::Language,
    node: &Node,
    source: &str,
    query_str: &str,
) -> Option<String> {
    let query = Query::new(&language, query_str)
        .ok()
        .expect("failed to instantiate query");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, *node, source.as_bytes());

    matches.next().and_then(|m| {
        m.captures
            .first()
            .and_then(|c| c.node.utf8_text(source.as_bytes()).ok().map(String::from))
    })
}

pub fn get_many(
    language: tree_sitter::Language,
    node: &Node,
    source: &str,
    query_str: &str,
) -> Vec<String> {
    let query = Query::new(&language, query_str)
        .ok()
        .expect("failed to instantiate query");

    let mut cursor = QueryCursor::new();

    let mut results = Vec::new();
    cursor
        .matches(&query, *node, source.as_bytes())
        .for_each(|m| {
            for capture in m.captures {
                if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                    results.push(text.to_string());
                }
            }
        });
    results
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
