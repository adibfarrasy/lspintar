use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

#[tracing::instrument(skip_all)]
pub fn extract_class_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (class_declaration
        (modifiers)? @modifiers
        name: (identifier) @class_name
        interfaces: (super_interfaces)? @interface_line
        superclass: (superclass)? @superclass_line
    )

    (_
        (groovydoc_comment) @groovydoc
        .
        (class_declaration
            (modifiers)? @modifiers
            name: (identifier) @class_name
            interfaces: (super_interfaces)? @interface_line
            superclass: (superclass)? @superclass_line
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut interface_line = String::new();
    let mut superclass_line = String::new();
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
                    "class_name" => {
                        if class_name.is_empty() {
                            class_name.push_str("class ");
                            class_name.push_str(text);
                        }
                    }
                    "interface_line" => {
                        if interface_line.is_empty() {
                            interface_line = text.to_string();
                        }
                    }
                    "superclass_line" => {
                        if superclass_line.is_empty() {
                            superclass_line = text.to_string();
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

    format_class_signature(
        package_name,
        modifiers,
        class_name,
        interface_line,
        superclass_line,
        groovydoc,
    )
}

fn format_class_signature(
    package_name: String,
    modifiers: String,
    class_name: String,
    interface_line: String,
    superclass_line: String,
    groovydoc: String,
) -> Option<String> {
    if class_name.is_empty() {
        return None;
    }

    use crate::languages::common::hover::partition_modifiers;
    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);

    // Build signature line with "class" keyword
    let mut signature_line = String::new();
    signature_line.push_str("class ");
    signature_line.push_str(&class_name);

    // Combine inheritance clauses
    let mut inheritance_parts = Vec::new();
    if !superclass_line.is_empty() {
        inheritance_parts.push(superclass_line);
    }
    if !interface_line.is_empty() {
        inheritance_parts.push(interface_line);
    }
    let inheritance = if inheritance_parts.is_empty() {
        None
    } else {
        Some(inheritance_parts.join(", "))
    };

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers_vec)
        .with_signature_line(signature_line)
        .with_inheritance(inheritance)
        .with_documentation(if groovydoc.is_empty() { None } else { Some(groovydoc) });

    Some(hover.format())
}
