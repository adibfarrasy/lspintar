use log::debug;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (
      (groovydoc_comment)? @groovydoc
      (field_declaration
        (modifiers)? @modifiers
        type: (type_identifier) @field_type
        declarator: (variable_declarator
          name: (identifier) @field_name
          value: (_)? @initial_value
        ))
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse field query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut modifiers = String::new();
    let mut field_type = String::new();
    let mut initial_value = String::new();
    let mut found_field = false;
    let mut groovydoc = String::new();

    let node_text = node.utf8_text(source.as_bytes()).ok()?;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found_field {
                return;
            }

            let mut current_field_name = String::new();
            let mut temp_modifiers = String::new();
            let mut temp_field_type = String::new();
            let mut temp_initial_value = String::new();

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name = text.to_string(),
                    "modifiers" => temp_modifiers = text.to_string(),
                    "field_type" => temp_field_type = text.to_string(),
                    "field_name" => current_field_name = text.to_string(),
                    "initial_value" => temp_initial_value = text.to_string(),
                    "groovydoc" => groovydoc = text.to_string(),
                    _ => {}
                }
            }

            if current_field_name == node_text {
                modifiers = temp_modifiers;
                field_type = temp_field_type;
                initial_value = temp_initial_value;
                found_field = true;
            }
        });

    if !found_field {
        return None;
    }

    format_field_signature(
        package_name,
        modifiers,
        field_type,
        node_text.to_string(),
        initial_value,
        groovydoc,
    )
}

fn format_field_signature(
    package_name: String,
    modifiers: String,
    field_type: String,
    field_name: String,
    initial_value: String,
    groovydoc: String,
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

    if !field_type.is_empty() {
        signature.push_str(&field_type);
        signature.push(' ');
    } else {
        signature.push_str("def ");
    }

    signature.push_str(&field_name);

    if !initial_value.is_empty() {
        signature.push_str(" = ");
        signature.push_str(&initial_value);
    }

    parts.push(signature);

    parts.push("```".to_string());
    parts.push("\n".to_string());

    parts.push("---".to_string());
    parts.push(groovydoc);

    Some(parts.join("\n"))
}
