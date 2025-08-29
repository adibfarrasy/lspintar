use log::debug;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

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
    use crate::languages::common::hover::partition_modifiers;
    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);

    let mut signature_line = String::new();
    
    if !field_type.is_empty() {
        signature_line.push_str(&field_type);
        signature_line.push(' ');
    } else {
        signature_line.push_str("def ");
    }

    signature_line.push_str(&field_name);

    if !initial_value.is_empty() {
        signature_line.push_str(" = ");
        signature_line.push_str(&initial_value);
    }

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers_vec)
        .with_signature_line(signature_line)
        .with_documentation(if groovydoc.is_empty() { None } else { Some(groovydoc) });

    Some(hover.format())
}
