use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Get the field name from the node
    let field_name = node.utf8_text(source.as_bytes()).ok()?;

    // Look for property declarations that match this field name
    let query_text = r#"
    (property_declaration
        (modifiers)? @modifiers
        (variable_declaration
            name: (simple_identifier) @field_name
            type: (user_type)? @field_type
        )
        ("=" (expression))? @initializer
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        let mut found_field = false;
        let mut modifiers = String::new();
        let mut field_type = String::new();
        let mut has_initializer = false;

        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "field_name" => {
                    if text == field_name {
                        found_field = true;
                    }
                }
                "modifiers" => {
                    modifiers.push_str(text);
                }
                "field_type" => {
                    field_type.push_str(text);
                }
                "initializer" => {
                    has_initializer = true;
                }
                _ => {}
            }
        }

        if found_field {
            let (access_modifiers, other_modifiers) = partition_modifiers(&modifiers);

            let mut signature = String::new();
            signature.push_str("```kotlin\n");

            if !access_modifiers.is_empty() {
                signature.push_str(&access_modifiers);
                signature.push(' ');
            }

            if !other_modifiers.is_empty() {
                signature.push_str(&other_modifiers);
                signature.push(' ');
            }

            // Determine if it's val or var based on modifiers
            if modifiers.contains("var") {
                signature.push_str("var ");
            } else {
                signature.push_str("val ");
            }

            signature.push_str(field_name);

            if !field_type.is_empty() {
                signature.push_str(": ");
                signature.push_str(&field_type);
            }

            if has_initializer {
                signature.push_str(" = ...");
            }

            signature.push_str("\n```");

            // Try to find the containing class for additional context
            if let Some(class_info) = find_containing_class(tree, node, source) {
                signature.push_str(&format!("\n\n**Declared in:** `{}`", class_info));
            }

            return Some(signature);
        }
    }

    // Fallback: look for class parameters that might be properties
    let param_query_text = r#"
    (primary_constructor
        (class_parameters
            (class_parameter
                (modifiers)? @modifiers
                name: (simple_identifier) @param_name
                type: (user_type) @param_type
            )
        )
    )
    "#;

    let param_query = Query::new(&tree.language(), param_query_text).ok()?;
    let mut param_cursor = QueryCursor::new();

    let mut param_matches = param_cursor.matches(&param_query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = param_matches.next() {
        let mut found_param = false;
        let mut modifiers = String::new();
        let mut param_type = String::new();

        for capture in query_match.captures.iter() {
            let capture_name = param_query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "param_name" => {
                    if text == field_name {
                        found_param = true;
                    }
                }
                "modifiers" => {
                    modifiers.push_str(text);
                }
                "param_type" => {
                    param_type.push_str(text);
                }
                _ => {}
            }
        }

        if found_param {
            let (access_modifiers, other_modifiers) = partition_modifiers(&modifiers);

            let mut signature = String::new();
            signature.push_str("```kotlin\n");

            if !access_modifiers.is_empty() {
                signature.push_str(&access_modifiers);
                signature.push(' ');
            }

            if !other_modifiers.is_empty() {
                signature.push_str(&other_modifiers);
                signature.push(' ');
            }

            // Check if it's val or var in modifiers, default to val for constructor parameters
            if modifiers.contains("var") {
                signature.push_str("var ");
            } else {
                signature.push_str("val ");
            }

            signature.push_str(field_name);
            signature.push_str(": ");
            signature.push_str(&param_type);

            signature.push_str("\n```");
            signature.push_str("\n\n*Constructor parameter property*");

            return Some(signature);
        }
    }

    None
}

fn find_containing_class(tree: &Tree, field_node: &Node, source: &str) -> Option<String> {
    // Walk up the tree from the field node to find the containing class
    let mut current = field_node.parent()?;

    while let Some(parent) = current.parent() {
        match current.kind() {
            "class_declaration" | "object_declaration" => {
                // Find the class/object name
                for child in current.children(&mut current.walk()) {
                    if child.kind() == "type_identifier" {
                        return child.utf8_text(source.as_bytes()).ok().map(String::from);
                    }
                }
            }
            _ => {}
        }
        current = parent;
    }

    None
}