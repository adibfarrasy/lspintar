use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

fn byte_to_position(source: &str, byte_offset: usize) -> Position {
    let mut line = 0;
    let mut character = 0;

    for (i, ch) in source.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

pub fn collect_syntax_errors(tree: &Tree, source: &str, lsp_source: &str) -> Vec<Diagnostic> {
    let query_text = r#"(ERROR) @error"#;
    let query = Query::new(&tree_sitter_groovy::language(), query_text).unwrap();
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut diagnostics = Vec::new();
    matches.for_each(|match_| {
        match_.captures.into_iter().for_each(|capture| {
            let node = capture.node;
            diagnostics.push(Diagnostic {
                range: Range {
                    start: byte_to_position(source, node.start_byte()),
                    end: byte_to_position(source, node.end_byte()),
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: "Syntax error".to_string(),
                source: Some(lsp_source.to_string()),
                ..Default::default()
            });
        });
    });
    diagnostics
}
