use log::debug;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

// FIXME: currently doesn't work
// 1. declaration in local file => cannot get the correct one due to early return in find_parent_method_invocation
// 2. declaration in another file =>
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (method_declaration
      (modifiers)? @modifiers
      type: (_)? @return_type
      name: (identifier) @method_name
      parameters: (formal_parameters) @parameters
      (throws)? @throws_clause
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse method query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut modifiers = String::new();
    let mut return_type = String::new();
    let mut parameters = String::new();
    let mut throws_clause = String::new();
    let mut found_method = false;

    let node_text = node.utf8_text(source.as_bytes()).ok()?;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found_method {
                return;
            }

            let mut current_method_name = String::new();
            let mut temp_modifiers = String::new();
            let mut temp_return_type = String::new();
            let mut temp_parameters = String::new();
            let mut temp_throws = String::new();

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name = text.to_string(),
                    "modifiers" => temp_modifiers = text.to_string(),
                    "return_type" => temp_return_type = text.to_string(),
                    "method_name" => current_method_name = text.to_string(),
                    "parameters" => temp_parameters = text.to_string(),
                    "throws_clause" => temp_throws = text.to_string(),
                    _ => {}
                }
            }

            if current_method_name == node_text {
                modifiers = temp_modifiers;
                return_type = temp_return_type;
                parameters = temp_parameters;
                throws_clause = temp_throws;
                found_method = true;
            }
        });

    if !found_method {
        return None;
    }

    format_method_signature(
        package_name,
        modifiers,
        return_type,
        node_text.to_string(),
        parameters,
        throws_clause,
    )
}

fn format_method_signature(
    package_name: String,
    modifiers: String,
    return_type: String,
    method_name: String,
    parameters: String,
    throws_clause: String,
) -> Option<String> {
    let mut parts = Vec::new();

    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```groovy".to_string());

    let (annotation, modifiers) = partition_modifiers(modifiers);
    annotation.into_iter().for_each(|a| parts.push(a));

    if !modifiers.is_empty() {
        let modifier_line = modifiers.join(" ");
        parts.push(modifier_line);
    }

    let mut signature = String::new();

    if !return_type.is_empty() {
        signature.push_str(&return_type);
        signature.push(' ');
    } else {
        signature.push_str("def ");
    }

    signature.push_str(&method_name);
    signature.push_str(&parameters);

    if !throws_clause.is_empty() {
        signature.push(' ');
        signature.push_str(&throws_clause);
    }

    parts.push(signature);
    parts.push("```".to_string());
    parts.push("\n".to_string());
    parts.push("---".to_string());
    parts.push("Method documentation".to_string());

    Some(parts.join("\n"))
}
