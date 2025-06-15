use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_interface_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (interface_declaration
      (modifiers)? @modifiers
      name: (identifier) @interface_name
      interfaces: (super_interfaces)? @extends_line
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut extends_line = String::new();
    let mut modifiers = String::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name.push_str(text),
                    "modifiers" => modifiers.push_str(text),
                    "interface_name" => {
                        if !interface_name.is_empty() {
                            interface_name.push(' ');
                        }
                        interface_name.push_str("interface ");
                        interface_name.push_str(text);
                    }
                    "extends_line" => extends_line = text.to_string(),
                    _ => {}
                }
            }
        });

    format_interface_signature(package_name, modifiers, interface_name, extends_line)
}

fn format_interface_signature(
    package_name: String,
    modifiers: String,
    interface_name: String,
    extends_line: String,
) -> Option<String> {
    if interface_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    parts.push(package_name);
    parts.push("\n".to_string());

    parts.push("```groovy".to_string());

    let (annotation, modifiers) = partition_modifiers(modifiers);
    annotation.into_iter().for_each(|a| parts.push(a));

    if !modifiers.is_empty() {
        let modifier_line = modifiers.join(" ");
        parts.push(modifier_line);
    }

    parts.push(interface_name);

    if !extends_line.is_empty() {
        parts.push(" ".to_string());
        parts.push(extends_line);
    }

    parts.push("```".to_string());
    parts.push("\n".to_string());

    parts.push("---".to_string());

    // TODO: Add docstring extraction
    parts.push("<placeholder docstring>".to_string());

    Some(parts.join("\n"))
}
