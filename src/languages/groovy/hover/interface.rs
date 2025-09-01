use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

/// Find the interface declaration node that contains or corresponds to the given node
fn find_target_interface_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "interface_declaration" => return Some(current_node),
            "identifier" => {
                // If we're hovering over an identifier, check if its parent is an interface_declaration
                if let Some(parent) = current_node.parent() {
                    if parent.kind() == "interface_declaration" {
                        return Some(parent);
                    }
                }
                current = current_node.parent();
            }
            _ => current = current_node.parent(),
        }
    }
    
    None
}

#[tracing::instrument(skip_all)]
pub fn extract_interface_signature(tree: &Tree, node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Find the target interface node
    let target_interface_node = find_target_interface_node(node)?;
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

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut extends_line = String::new();
    let mut modifiers = String::new();
    let mut groovydoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        // Check if this match corresponds to our target interface
        let mut is_target_interface_match = false;
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "interface_name" {
                // Check if this interface_name capture is within our target interface node
                if capture.node.start_byte() >= target_interface_node.start_byte() 
                   && capture.node.end_byte() <= target_interface_node.end_byte() {
                    is_target_interface_match = true;
                    break;
                }
            }
        }

        // Only process captures for our target interface match
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
                    if is_target_interface_match && modifiers.is_empty() {
                        modifiers = text.to_string();
                    }
                }
                "interface_name" => {
                    if is_target_interface_match && interface_name.is_empty() {
                        interface_name.push_str(text);
                    }
                }
                "extends_line" => {
                    if is_target_interface_match && extends_line.is_empty() {
                        extends_line = text.to_string();
                    }
                }
                "groovydoc" => {
                    if is_target_interface_match && groovydoc.is_empty() {
                        groovydoc = text.to_string();
                    }
                }
                _ => {}
            }
        }
    }

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
