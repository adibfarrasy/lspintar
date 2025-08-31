use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{partition_modifiers, HoverSignature};

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Get the method name from the node
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    // Look for function declarations that match this method name
    let query_text = r#"
    (
        (multiline_comment)? @kdoc
        (function_declaration
            (modifiers)? @modifiers
            (simple_identifier) @method_name
            (type_parameters)? @type_params
            (function_value_parameters) @parameters
            (user_type)? @return_type
        )
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
        let mut kdoc = String::new();

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
                "kdoc" => {
                    if kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }

        if found_method {
            let (annotations, modifier_vec) = partition_modifiers(&modifiers);

            // Build signature line
            let mut signature_line = String::new();
            signature_line.push_str("fun ");

            if !type_params.is_empty() {
                signature_line.push_str(&type_params);
                signature_line.push(' ');
            }

            signature_line.push_str(method_name);
            signature_line.push_str(&parameters.replace('\n', " "));

            if !return_type.is_empty() {
                signature_line.push_str(": ");
                signature_line.push_str(&return_type);
            }

            let mut hover = HoverSignature::new("kotlin")
                .with_annotations(annotations)
                .with_modifiers(modifier_vec)
                .with_signature_line(signature_line)
                .with_documentation(if kdoc.is_empty() { None } else { Some(kdoc) });

            // Try to find the containing class for additional context
            if let Some(class_info) = find_containing_class(tree, node, source) {
                hover = hover.add_info(format!("**Declared in:** `{}`", class_info));
            }

            return Some(hover.format());
        }
    }

    // Look for constructor declarations as well
    let constructor_query_text = r#"
    (secondary_constructor
        (modifiers)? @modifiers
        (function_value_parameters) @parameters
    )
    "#;

    let constructor_query = Query::new(&tree.language(), constructor_query_text).ok()?;
    let mut constructor_cursor = QueryCursor::new();

    let mut constructor_matches =
        constructor_cursor.matches(&constructor_query, tree.root_node(), source.as_bytes());

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

            let (annotations, modifier_vec) = partition_modifiers(&modifiers);

            let mut parts = Vec::new();
            parts.push("```kotlin".to_string());

            // Add annotations (each on separate lines)
            annotations.into_iter().for_each(|a| parts.push(a));

            // Build the constructor declaration line
            let mut constructor_line = String::new();

            if !modifier_vec.is_empty() {
                constructor_line.push_str(&modifier_vec.join(" "));
                constructor_line.push(' ');
            }

            constructor_line.push_str("constructor");
            constructor_line.push_str(&parameters.replace('\n', " "));

            parts.push(constructor_line);
            parts.push("```".to_string());
            parts.push("\n*Secondary constructor*".to_string());

            return Some(parts.join("\n"));
        }
    }

    None
}

fn find_containing_class(_tree: &Tree, method_node: &Node, source: &str) -> Option<String> {
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

fn is_node_within_constructor(
    constructor_match: &tree_sitter::QueryMatch,
    target_node: &Node,
) -> bool {
    for capture in constructor_match.captures.iter() {
        let constructor_node = capture.node;

        // Check if target_node is within the range of constructor_node
        if target_node.start_byte() >= constructor_node.start_byte()
            && target_node.end_byte() <= constructor_node.end_byte()
        {
            return true;
        }
    }

    false
}

