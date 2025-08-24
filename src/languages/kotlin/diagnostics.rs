use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tree_sitter::{Node, Tree};

pub fn collect_syntax_errors(tree: &Tree, source: &str, lsp_name: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    collect_syntax_errors_recursive(tree.root_node(), source, &mut diagnostics, lsp_name);
    diagnostics
}

fn collect_syntax_errors_recursive(
    node: Node,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
    lsp_name: &str,
) {
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
                code: None,
                code_description: None,
                source: Some(lsp_name.to_string()),
                message,
                related_information: None,
                tags: None,
                data: None,
            });
        }

        // Continue checking children for more errors
        for child in node.children(&mut node.walk()) {
            collect_syntax_errors_recursive(child, source, diagnostics, lsp_name);
        }
    }
}