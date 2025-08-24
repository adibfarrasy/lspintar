use log::debug;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Find the field declaration node that contains this node
    let field_node = find_field_node(node)?;
    
    let query_text = r#"
    (
      (block_comment)? @javadoc
      (field_declaration
        (modifiers)? @modifiers
        type: (_) @field_type
        declarator: (variable_declarator
          name: (identifier) @field_name
          value: (_)? @field_value
        )
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse field query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut field_name = String::new();
    let mut field_type = String::new();
    let mut field_value = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, field_node, source.as_bytes());
    
    // Take only the first field match
    if let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "modifiers" => modifiers.push_str(text),
                "field_type" => field_type.push_str(text),
                "field_name" => field_name.push_str(text),
                "field_value" => field_value = text.to_string(),
                "javadoc" => javadoc = text.to_string(),
                _ => {}
            }
        }
    }

    format_field_signature(
        modifiers,
        field_type,
        field_name,
        field_value,
        javadoc,
    )
}

fn find_field_node<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "field_declaration" => return Some(current_node),
            _ => current = current_node.parent(),
        }
    }
    
    None
}

fn format_field_signature(
    modifiers: String,
    field_type: String,
    field_name: String,
    field_value: String,
    javadoc: String,
) -> Option<String> {
    if field_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    parts.push("```java".to_string());

    let (annotation, modifiers) = partition_modifiers(modifiers);
    annotation.into_iter().for_each(|a| parts.push(a));

    let mut signature_line = String::new();

    if !modifiers.is_empty() {
        signature_line.push_str(&modifiers.join(" "));
        signature_line.push(' ');
    }

    if !field_type.is_empty() {
        signature_line.push_str(&field_type);
        signature_line.push(' ');
    }

    signature_line.push_str(&field_name);

    if !field_value.is_empty() {
        signature_line.push_str(" = ");
        signature_line.push_str(&field_value);
    }

    parts.push(signature_line);
    parts.push("```".to_string());

    if !javadoc.is_empty() {
        parts.push("\n".to_string());
        parts.push("---".to_string());
        parts.push(javadoc);
    }

    Some(parts.join("\n"))
}