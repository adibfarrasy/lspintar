use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};
use crate::core::constants::KOTLIN_PARSER;

pub fn extract_call_signature_from_context(usage_node: &Node, source: &str) -> Option<CallSignature> {
    // Find the call expression that contains this usage
    let call_node = find_parent_call_expression(usage_node)?;
    
    // Extract parameter information from the call
    extract_call_signature_from_call_node(&call_node, source)
}

fn find_parent_call_expression<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = *node;
    
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "call_expression" | "navigation_expression" => return Some(parent),
            _ => current = parent,
        }
    }
    
    None
}

fn extract_call_signature_from_call_node(call_node: &Node, _source: &str) -> Option<CallSignature> {
    let mut parameter_count = 0;
    
    // Look for value_arguments in the call expression
    for child in call_node.children(&mut call_node.walk()) {
        if child.kind() == "value_arguments" {
            // Count each argument
            for arg_child in child.children(&mut child.walk()) {
                if arg_child.kind() == "value_argument" {
                    parameter_count += 1;
                }
            }
        }
    }
    
    Some(CallSignature {
        parameter_count,
    })
}


/// Find method declarations that match the given signature
pub fn find_method_with_signature<'a>(
    tree: &'a tree_sitter::Tree,
    source: &str,
    method_name: &str,
    signature: &CallSignature,
) -> Option<Node<'a>> {
    let query_text = r#"(function_declaration (simple_identifier) @name)"#;
    
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == method_name {
                    // Found a method with matching name, check if signature matches
                    if let Some(function_node) = capture.node.parent() {
                        if method_signature_matches(&function_node, source, signature) {
                            return Some(function_node);
                        }
                    }
                }
            }
        }
    }
    
    None
}

fn method_signature_matches(function_node: &Node, _source: &str, call_signature: &CallSignature) -> bool {
    // Count parameters in the function declaration
    let mut param_count = 0;
    
    for child in function_node.children(&mut function_node.walk()) {
        if child.kind() == "function_value_parameters" {
            for param_child in child.children(&mut child.walk()) {
                if param_child.kind() == "parameter" {
                    param_count += 1;
                }
            }
        }
    }
    
    // For now, just match parameter count
    // In a more sophisticated implementation, we could also match parameter types
    param_count == call_signature.parameter_count
}

#[derive(Debug, Clone)]
pub struct CallSignature {
    pub parameter_count: usize,
}