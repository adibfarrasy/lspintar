use log::debug;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Find the method declaration node that contains this node
    let method_node = find_method_node(node)?;
    
    let query_text = r#"
    (
      (block_comment)? @javadoc
      (method_declaration
        (modifiers)? @modifiers
        type: (_) @return_type
        name: (identifier) @method_name
        parameters: (formal_parameters) @parameters
        (throws)? @throws_clause
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse method query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut method_name = String::new();
    let mut parameters = String::new();
    let mut return_type = String::new();
    let mut modifiers = String::new();
    let mut throws_clause = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, method_node, source.as_bytes());
    
    // Take only the first method match
    if let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "modifiers" => modifiers.push_str(text),
                "return_type" => return_type.push_str(text),
                "method_name" => method_name.push_str(text),
                "parameters" => parameters.push_str(text),
                "throws_clause" => throws_clause = text.to_string(),
                "javadoc" => javadoc = text.to_string(),
                _ => {}
            }
        }
    }

    format_method_signature(
        modifiers,
        return_type,
        method_name,
        parameters,
        throws_clause,
        javadoc,
    )
}

fn find_method_node<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "method_declaration" | "constructor_declaration" => return Some(current_node),
            _ => current = current_node.parent(),
        }
    }
    
    None
}

fn format_method_signature(
    modifiers: String,
    return_type: String,
    method_name: String,
    parameters: String,
    throws_clause: String,
    javadoc: String,
) -> Option<String> {
    if method_name.is_empty() {
        return None;
    }

    use crate::languages::common::hover::partition_modifiers;
    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);

    let mut signature_line = String::new();
    
    if !return_type.is_empty() {
        signature_line.push_str(&return_type);
        signature_line.push(' ');
    }

    signature_line.push_str(&method_name);
    signature_line.push_str(&parameters);

    if !throws_clause.is_empty() {
        signature_line.push(' ');
        signature_line.push_str(&throws_clause);
    }

    let hover = HoverSignature::new("java")
        .with_annotations(annotations)
        .with_modifiers(modifiers_vec)
        .with_signature_line(signature_line)
        .with_documentation(if javadoc.is_empty() { None } else { Some(javadoc) });

    Some(hover.format())
}