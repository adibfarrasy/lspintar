use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{
    format_inheritance, parse_parameters, format_parameters, deduplicate_modifiers, HoverSignature,
};

/// Find the class declaration node that contains or corresponds to the given node
fn find_target_class_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "class_declaration" => return Some(current_node),
            "type_identifier" => {
                // If we're hovering over a type_identifier, check if its parent is a class_declaration
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
    (package_header (identifier) @package_name)

    (class_declaration
        (modifiers 
            (annotation)* @annotation
            (visibility_modifier)* @modifier
            (class_modifier)* @modifier
        )?
        (type_identifier) @class_name
        (type_parameters)? @type_params
        (primary_constructor)? @primary_constructor
        (delegation_specifier)* @supertypes
    )

    (_
        (multiline_comment) @kdoc
        .
        (class_declaration
            (modifiers 
                (annotation)* @annotation
                (visibility_modifier)* @modifier
                (class_modifier)* @modifier
            )?
            (type_identifier) @class_name
            (type_parameters)? @type_params
            (primary_constructor)? @primary_constructor
            (delegation_specifier)* @supertypes
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut type_params = String::new();
    let mut primary_constructor = String::new();
    let mut supertypes = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut kdoc = String::new();

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
                "type_params" => {
                    if is_target_class_match && type_params.is_empty() {
                        type_params.push_str(text);
                    }
                }
                "primary_constructor" => {
                    if is_target_class_match && primary_constructor.is_empty() {
                        primary_constructor.push_str(text);
                    }
                }
                "supertypes" => {
                    if is_target_class_match && supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                "kdoc" => {
                    if is_target_class_match && kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if class_name.is_empty() {
        return None;
    }

    // Find the comment that immediately precedes this class
    let kdoc = find_preceding_comment(tree, source, &class_name);

    // Parse constructor parameters and format them according to ≤3 vs >3 rule
    let param_list = parse_parameters(&primary_constructor);
    let formatted_params = format_parameters(&param_list);

    // Build signature line with integrated constructor
    let mut signature_line = String::new();
    signature_line.push_str("class ");
    signature_line.push_str(&class_name);

    if !type_params.is_empty() {
        signature_line.push_str(&type_params.replace('\n', " "));
    }

    // Add constructor parameters inline for ≤3, or format will handle multi-line for >3
    if !formatted_params.is_empty() && formatted_params != "()" {
        signature_line.push_str(&formatted_params);
    }

    // Deduplicate modifiers to avoid "data data data" issue
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

/// Find a comment that immediately precedes the class declaration
fn find_preceding_comment(tree: &Tree, source: &str, class_name: &str) -> String {
    let root = tree.root_node();
    
    // Find the class declaration node
    let class_node = find_class_node(&root, source, class_name);
    if let Some(class_node) = class_node {
        // Look for a preceding sibling that is a multiline_comment
        if let Some(parent) = class_node.parent() {
            let mut cursor = parent.walk();
            if cursor.goto_first_child() {
                let mut prev_node: Option<tree_sitter::Node> = None;
                loop {
                    let current = cursor.node();
                    if current == class_node {
                        // Found our class, check if previous node was a comment
                        if let Some(prev) = prev_node {
                            if prev.kind() == "multiline_comment" {
                                if let Ok(comment_text) = prev.utf8_text(source.as_bytes()) {
                                    return comment_text.to_string();
                                }
                            }
                        }
                        break;
                    }
                    prev_node = Some(current);
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
    
    String::new()
}

/// Find the class declaration node by name
fn find_class_node<'a>(node: &tree_sitter::Node<'a>, source: &str, class_name: &str) -> Option<tree_sitter::Node<'a>> {
    if node.kind() == "class_declaration" {
        // Check if this is the class we're looking for
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "type_identifier" {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        if text == class_name {
                            return Some(*node);
                        }
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    
    // Recursively search children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(found) = find_class_node(&cursor.node(), source, class_name) {
                return Some(found);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    
    None
}

