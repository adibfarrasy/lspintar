use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Get the method name from the node
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    // Look for function declarations that match this method name
    let query_text = r#"
    (function_declaration
        (modifiers)? @modifiers
        name: (simple_identifier) @method_name
        (type_parameters)? @type_params
        parameters: (function_value_parameters) @parameters
        type: (user_type)? @return_type
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        let mut found_method = false;
        let mut modifiers = String::new();
        let mut type_params = String::new();
        let mut parameters = String::new();
        let mut return_type = String::new();

        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "method_name" => {
                    if text == method_name {
                        found_method = true;
                    }
                }
                "modifiers" => {
                    modifiers.push_str(text);
                }
                "type_params" => {
                    type_params.push_str(text);
                }
                "parameters" => {
                    parameters.push_str(text);
                }
                "return_type" => {
                    return_type.push_str(text);
                }
                _ => {}
            }
        }

        if found_method {
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

            signature.push_str("fun ");

            if !type_params.is_empty() {
                signature.push_str(&type_params);
                signature.push(' ');
            }

            signature.push_str(method_name);
            signature.push_str(&parameters.replace('\n', " "));

            if !return_type.is_empty() {
                signature.push_str(": ");
                signature.push_str(&return_type);
            }

            signature.push_str("\n```");

            // Try to find the containing class for additional context
            if let Some(class_info) = find_containing_class(tree, node, source) {
                signature.push_str(&format!("\n\n**Declared in:** `{}`", class_info));
            }

            return Some(signature);
        }
    }

    // Look for constructor declarations as well
    let constructor_query_text = r#"
    (secondary_constructor
        (modifiers)? @modifiers
        parameters: (function_value_parameters) @parameters
    )
    "#;

    let constructor_query = Query::new(&tree.language(), constructor_query_text).ok()?;
    let mut constructor_cursor = QueryCursor::new();

    let mut constructor_matches = constructor_cursor.matches(&constructor_query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = constructor_matches.next() {
        // Check if the node is within this constructor
        if is_node_within_constructor(&query_match, node) {
            let mut modifiers = String::new();
            let mut parameters = String::new();

            for capture in query_match.captures.iter() {
                let capture_name = constructor_query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "modifiers" => {
                        modifiers.push_str(text);
                    }
                    "parameters" => {
                        parameters.push_str(text);
                    }
                    _ => {}
                }
            }

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

            signature.push_str("constructor");
            signature.push_str(&parameters.replace('\n', " "));

            signature.push_str("\n```");
            signature.push_str("\n\n*Secondary constructor*");

            return Some(signature);
        }
    }

    None
}

fn find_containing_class(tree: &Tree, method_node: &Node, source: &str) -> Option<String> {
    // Walk up the tree from the method node to find the containing class
    let mut current = method_node.parent()?;

    while let Some(parent) = current.parent() {
        match current.kind() {
            "class_declaration" | "object_declaration" | "interface_declaration" => {
                // Find the class/object/interface name
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

fn is_node_within_constructor(constructor_match: &tree_sitter::QueryMatch, target_node: &Node) -> bool {
    for capture in constructor_match.captures.iter() {
        let constructor_node = capture.node;
        
        // Check if target_node is within the range of constructor_node
        if target_node.start_byte() >= constructor_node.start_byte() 
            && target_node.end_byte() <= constructor_node.end_byte() {
            return true;
        }
    }
    
    false
}