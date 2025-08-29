use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

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

    (_
        (groovydoc_comment) @groovydoc
        .
        (interface_declaration
            (modifiers)? @modifiers
            name: (identifier) @interface_name
            interfaces: (super_interfaces)? @extends_line
        )
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
    let mut groovydoc = String::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => {
                        if package_name.is_empty() {
                            package_name.push_str(text);
                        }
                    }
                    "modifiers" => {
                        if modifiers.is_empty() {
                            modifiers = text.to_string();
                        }
                    }
                    "interface_name" => {
                        if interface_name.is_empty() {
                            interface_name.push_str("interface ");
                            interface_name.push_str(text);
                        }
                    }
                    "extends_line" => {
                        if extends_line.is_empty() {
                            extends_line = text.to_string();
                        }
                    }
                    "groovydoc" => {
                        if groovydoc.is_empty() {
                            groovydoc = text.to_string();
                        }
                    }
                    _ => {}
                }
            }
        });

    format_interface_signature(
        package_name,
        modifiers,
        interface_name,
        extends_line,
        groovydoc,
    )
}

fn format_interface_signature(
    package_name: String,
    modifiers: String,
    interface_name: String,
    extends_line: String,
    groovydoc: String,
) -> Option<String> {
    if interface_name.is_empty() {
        return None;
    }

    use crate::languages::common::hover::partition_modifiers;
    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("interface ");
    signature_line.push_str(&interface_name);

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers_vec)
        .with_signature_line(signature_line)
        .with_inheritance(if extends_line.is_empty() { None } else { Some(extends_line) })
        .with_documentation(if groovydoc.is_empty() { None } else { Some(groovydoc) });

    Some(hover.format())
}
