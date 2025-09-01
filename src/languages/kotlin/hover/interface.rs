use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{format_inheritance, deduplicate_modifiers, HoverSignature};

/// Find the interface declaration node that contains or corresponds to the given node
#[tracing::instrument(skip_all)]
fn find_target_interface_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "interface_declaration" => return Some(current_node),
            "type_identifier" => {
                // If we're hovering over a type_identifier, check if its parent is an interface_declaration
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
    (package_header (identifier) @package_name)

    (interface_declaration
        (modifiers 
            (annotation)* @annotation
            (visibility_modifier)* @modifier
            (class_modifier)* @modifier
            (function_modifier)* @modifier
        )?
        (type_identifier) @interface_name
        (type_parameters)? @type_params
        (delegation_specifier)* @supertypes
    )

    (_
        (multiline_comment) @kdoc
        .
        (interface_declaration
            (modifiers 
                (annotation)* @annotation
                (visibility_modifier)* @modifier
                (class_modifier)* @modifier
                (function_modifier)* @modifier
            )?
            (type_identifier) @interface_name
            (type_parameters)? @type_params
            (delegation_specifier)* @supertypes
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut type_params = String::new();
    let mut supertypes = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut kdoc = String::new();

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
                "annotation" => {
                    if is_target_interface_match {
                        annotations.push(text.to_string());
                    }
                }
                "modifier" => {
                    if is_target_interface_match {
                        modifiers.push(text.to_string());
                    }
                }
                "interface_name" => {
                    if is_target_interface_match && interface_name.is_empty() {
                        interface_name.push_str(text);
                    }
                }
                "type_params" => {
                    if is_target_interface_match && type_params.is_empty() {
                        type_params.push_str(text);
                    }
                }
                "supertypes" => {
                    if is_target_interface_match && supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                "kdoc" => {
                    if is_target_interface_match && kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if interface_name.is_empty() {
        return None;
    }

    // Annotations and modifiers are already separated by the query

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("interface ");
    signature_line.push_str(&interface_name);

    if !type_params.is_empty() {
        signature_line.push_str(&type_params.replace('\n', " "));
    }

    // Deduplicate modifiers to avoid repetition
    let unique_modifiers = deduplicate_modifiers(modifiers);

    let hover = HoverSignature::new("kotlin")
        .with_package(if package_name.is_empty() {
            None
        } else {
            Some(package_name)
        })
        .with_annotations(annotations)
        .with_modifiers(unique_modifiers)
        .with_signature_line(signature_line)
        .with_inheritance(format_inheritance(&supertypes))
        .with_documentation(if kdoc.is_empty() { None } else { Some(kdoc) });

    Some(hover.format())
}
