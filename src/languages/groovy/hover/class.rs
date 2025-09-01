use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

/// Find the class declaration node that contains or corresponds to the given node
fn find_target_class_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "class_declaration" => return Some(current_node),
            "identifier" => {
                // If we're hovering over an identifier, check if its parent is a class_declaration
                if let Some(parent) = current_node.parent() {
                    if parent.kind() == "class_declaration" {
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
pub fn extract_class_signature(tree: &Tree, node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Find the target class node
    let target_class_node = find_target_class_node(node)?;
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (class_declaration
        (modifiers 
            (annotation)* @annotation
            "public"? @modifier
            "private"? @modifier  
            "protected"? @modifier
            "static"? @modifier
            "final"? @modifier
            "abstract"? @modifier
        )?
        name: (identifier) @class_name
        interfaces: (super_interfaces)? @interface_line
        superclass: (superclass)? @superclass_line
    )

    (_
        (groovydoc_comment) @groovydoc
        .
        (class_declaration
            (modifiers 
                (annotation)* @annotation
                "public"? @modifier
                "private"? @modifier  
                "protected"? @modifier
                "static"? @modifier
                "final"? @modifier
                "abstract"? @modifier
            )?
            name: (identifier) @class_name
            interfaces: (super_interfaces)? @interface_line
            superclass: (superclass)? @superclass_line
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut interface_line = String::new();
    let mut superclass_line = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut groovydoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        // Check if this match corresponds to our target class
        let mut is_target_class_match = false;
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "class_name" {
                // Check if this class_name capture is within our target class node
                if capture.node.start_byte() >= target_class_node.start_byte() 
                   && capture.node.end_byte() <= target_class_node.end_byte() {
                    is_target_class_match = true;
                    break;
                }
            }
        }

        // Only process captures for our target class match
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                }
                "annotation" => {
                    if is_target_class_match {
                        annotations.push(text.to_string());
                    }
                }
                "modifier" => {
                    if is_target_class_match {
                        modifiers.push(text.to_string());
                    }
                }
                "class_name" => {
                    if is_target_class_match && class_name.is_empty() {
                        class_name.push_str(text);
                    }
                }
                "interface_line" => {
                    if is_target_class_match && interface_line.is_empty() {
                        interface_line = text.to_string();
                    }
                }
                "superclass_line" => {
                    if is_target_class_match && superclass_line.is_empty() {
                        superclass_line = text.to_string();
                    }
                }
                "groovydoc" => {
                    if is_target_class_match && groovydoc.is_empty() {
                        groovydoc = text.to_string();
                    }
                }
                _ => {}
            }
        }
    }

    format_class_signature(
        package_name,
        annotations,
        modifiers,
        class_name,
        interface_line,
        superclass_line,
        groovydoc,
    )
}

fn format_class_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    class_name: String,
    interface_line: String,
    superclass_line: String,
    groovydoc: String,
) -> Option<String> {
    if class_name.is_empty() {
        return None;
    }

    // Build signature line with "class" keyword
    let mut signature_line = String::new();
    signature_line.push_str("class ");
    signature_line.push_str(&class_name);

    // Separate inheritance clauses - extends and implements on separate lines
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
        Some(inheritance_parts.join("\n"))
    };

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers)
        .with_signature_line(signature_line)
        .with_inheritance(inheritance)
        .with_documentation(if groovydoc.is_empty() { None } else { Some(groovydoc) });

    Some(hover.format())
}
