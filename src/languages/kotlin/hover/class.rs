use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{
    format_inheritance, parse_constructor_params, HoverSignature,
};

#[tracing::instrument(skip_all)]
pub fn extract_class_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)

    (class_declaration
        (type_identifier) @class_name
        (type_parameters)? @type_params
        (primary_constructor)? @primary_constructor
        (delegation_specifier)* @supertypes
    )


    (class_declaration (modifiers (annotation) @annotation))

    (class_declaration (modifiers (visibility_modifier) @modifier))

    (class_declaration (modifiers (class_modifier) @modifier))

    (multiline_comment) @kdoc
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut type_params = String::new();
    let mut primary_constructor = String::new();
    let mut supertypes = String::new();
    let mut annotations = std::collections::HashSet::new();
    let mut modifiers = std::collections::HashSet::new();
    let mut kdoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
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
                    annotations.insert(text.to_string());
                }
                "modifier" => {
                    modifiers.insert(text.to_string());
                }
                "class_name" => {
                    if class_name.is_empty() {
                        class_name.push_str(text);
                    }
                }
                "type_params" => {
                    if type_params.is_empty() {
                        type_params.push_str(text);
                    }
                }
                "primary_constructor" => {
                    if primary_constructor.is_empty() {
                        primary_constructor.push_str(text);
                    }
                }
                "supertypes" => {
                    if supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                "kdoc" => {
                    if kdoc.is_empty() {
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

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("class ");
    signature_line.push_str(&class_name);

    if !type_params.is_empty() {
        signature_line.push_str(&type_params.replace('\n', " "));
    }

    let hover = HoverSignature::new("kotlin")
        .with_package(if package_name.is_empty() {
            None
        } else {
            Some(package_name)
        })
        .with_annotations(annotations.into_iter().collect())
        .with_modifiers(modifiers.into_iter().collect())
        .with_signature_line(signature_line)
        .with_constructor_params(parse_constructor_params(&primary_constructor))
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

