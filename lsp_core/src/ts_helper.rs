use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
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
    let mut matches = cursor.matches(query, *node, content.as_bytes());
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
        .matches(query, *node, content.as_bytes())
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

    let (type_and_name, default) = if let Some(eq_pos) = find_top_level_eq(param) {
        let (left, right) = param.split_at(eq_pos);
        (left.trim(), Some(right[1..].trim().trim_matches('\'')))
    } else {
        (param, None)
    };

    let type_and_name = strip_leading_annotations(type_and_name);

    if let Some(last_space) = type_and_name.rfind(char::is_whitespace) {
        let type_name = type_and_name[..last_space].trim().to_string();
        let name = type_and_name[last_space..].trim().to_string();
        (name, Some(type_name), default.map(String::from))
    } else {
        // No whitespace = untyped parameter (for Groovy)
        (type_and_name.to_string(), None, default.map(String::from))
    }
}

fn find_top_level_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut paren = 0i32;
    let mut bracket = 0i32;
    let mut brace = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'[' => bracket += 1,
            b']' => bracket -= 1,
            b'{' => brace += 1,
            b'}' => brace -= 1,
            b'"' | b'\'' => {
                let quote = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            b'=' if paren == 0 && bracket == 0 && brace == 0 => {
                // skip ==, >=, <=, !=
                let next = bytes.get(i + 1).copied();
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
                if next != Some(b'=')
                    && prev != Some(b'=')
                    && prev != Some(b'>')
                    && prev != Some(b'<')
                    && prev != Some(b'!')
                {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_leading_annotations(s: &str) -> &str {
    let mut s = s.trim_start();
    while let Some(rest) = s.strip_prefix('@') {
        let id_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        let mut rest = &rest[id_end..];
        let after_ws = rest.trim_start();
        if after_ws.starts_with('(') {
            let bytes = after_ws.as_bytes();
            let mut depth = 0i32;
            let mut end = None;
            for (i, &b) in bytes.iter().enumerate() {
                if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
            }
            match end {
                Some(e) => rest = &after_ws[e..],
                None => return s,
            }
        }
        let next = rest.trim_start();
        if next == s {
            return s;
        }
        s = next;
    }
    s
}

pub fn node_contains_position(node: &Node, position: &Position) -> bool {
    let (start, end) = (node.start_position(), node.end_position());
    let (line, char) = (position.line as usize, position.character as usize);

    (start.row < line || (start.row == line && start.column <= char))
        && (line < end.row || (line == end.row && char <= end.column))
}

pub fn position_to_byte_offset(content: &str, position: &Position) -> usize {
    let mut line = 0usize;
    let mut col = 0usize;
    let mut byte = 0usize;
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
        byte += ch.len_utf8();
    }
    byte
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

pub fn collect_syntax_errors(node: Node, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    if node.has_error() {
        if node.is_error() || node.is_missing() {
            let start_position = Position {
                line: node.start_position().row as u32,
                character: node.start_position().column as u32,
            };
            let end_position = Position {
                line: node.end_position().row as u32,
                character: node.end_position().column as u32,
            };

            let range = Range {
                start: start_position,
                end: end_position,
            };

            let message = if node.is_missing() {
                format!("Missing {}", node.kind())
            } else {
                let node_text = node.utf8_text(source.as_bytes()).unwrap_or("<unknown>");
                format!("Syntax error: unexpected '{}'", node_text)
            };

            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("syntax_error".to_string())),
                code_description: None,
                source: Some("lspintar".to_string()),
                message,
                related_information: None,
                tags: None,
                data: None,
            });
        }

        // Continue checking children for more errors
        for child in node.children(&mut node.walk()) {
            collect_syntax_errors(child, source, diagnostics);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_typed_parameter() {
        let (name, type_name, default) = parse_parameter("String snapshotId");
        assert_eq!(name, "snapshotId");
        assert_eq!(type_name.as_deref(), Some("String"));
        assert!(default.is_none());
    }

    #[test]
    fn strips_annotation_with_string_arg() {
        let (name, type_name, _) =
            parse_parameter("@PathVariable(\"snapshotId\") String snapshotId");
        assert_eq!(name, "snapshotId");
        assert_eq!(type_name.as_deref(), Some("String"));
    }

    #[test]
    fn strips_multiple_annotations() {
        let (name, type_name, _) =
            parse_parameter("@Valid @RequestBody UserInfoSnapshot snapshot");
        assert_eq!(name, "snapshot");
        assert_eq!(type_name.as_deref(), Some("UserInfoSnapshot"));
    }

    #[test]
    fn strips_annotation_with_nested_parens_and_eq() {
        let (name, type_name, default) =
            parse_parameter("@RequestParam(defaultValue = \"f(x)\") int count");
        assert_eq!(name, "count");
        assert_eq!(type_name.as_deref(), Some("int"));
        assert!(default.is_none());
    }

    #[test]
    fn strips_annotation_with_default_value() {
        let (name, type_name, default) =
            parse_parameter("@RequestParam String page = '1'");
        assert_eq!(name, "page");
        assert_eq!(type_name.as_deref(), Some("String"));
        assert_eq!(default.as_deref(), Some("1"));
    }

    #[test]
    fn untyped_parameter_unchanged() {
        let (name, type_name, _) = parse_parameter("snapshotId");
        assert_eq!(name, "snapshotId");
        assert!(type_name.is_none());
    }
}
