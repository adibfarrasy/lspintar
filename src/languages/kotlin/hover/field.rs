use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Get the field name from the node
    let field_name = node.utf8_text(source.as_bytes()).ok()?;

    // Look for property declarations that match this field name
    let query_text = r#"
    (
        (multiline_comment)? @kdoc
        (property_declaration
            (modifiers 
                (annotation)* @annotation
                (visibility_modifier)* @modifier
                (member_modifier)* @modifier
                (property_modifier)* @modifier
            )?
            (variable_declaration
                (simple_identifier) @field_name
                (user_type)? @field_type
            )
            ("=" (_))? @initializer
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        let mut found_field = false;
        let mut annotations = Vec::new();
        let mut modifiers = Vec::new();
        let mut field_type = String::new();
        let mut has_initializer = false;
        let mut kdoc = String::new();

        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "field_name" => {
                    if text == field_name {
                        found_field = true;
                    }
                }
                "annotation" => {
                    annotations.push(text.to_string());
                }
                "modifier" => {
                    modifiers.push(text.to_string());
                }
                "field_type" => {
                    field_type.push_str(text);
                }
                "initializer" => {
                    has_initializer = true;
                }
                "kdoc" => {
                    if kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }

        if found_field {
            // Build the field declaration line
            let mut field_line = String::new();

            // Determine if it's val or var based on modifiers
            if modifiers.iter().any(|m| m == "var") {
                field_line.push_str("var ");
            } else {
                field_line.push_str("val ");
            }

            field_line.push_str(field_name);

            if !field_type.is_empty() {
                field_line.push_str(": ");
                field_line.push_str(&field_type);
            }

            if has_initializer {
                field_line.push_str(" = ...");
            }

            let hover = HoverSignature::new("kotlin")
                .with_annotations(annotations)
                .with_modifiers(modifiers)
                .with_signature_line(field_line)
                .with_documentation(if kdoc.is_empty() { None } else { Some(kdoc) });

            return Some(hover.format());
        }
    }

    // Fallback: look for class parameters that might be properties
    let param_query_text = r#"
    (primary_constructor
        (class_parameters
            (class_parameter
                (modifiers 
                    (annotation)* @annotation
                    (visibility_modifier)* @modifier
                    (member_modifier)* @modifier
                    (property_modifier)* @modifier
                )?
                (simple_identifier) @param_name
                (user_type) @param_type
            )
        )
    )
    "#;

    let param_query = Query::new(&tree.language(), param_query_text).ok()?;
    let mut param_cursor = QueryCursor::new();

    let mut param_matches = param_cursor.matches(&param_query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = param_matches.next() {
        let mut found_param = false;
        let mut annotations = Vec::new();
        let mut modifiers = Vec::new();
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
                "annotation" => {
                    annotations.push(text.to_string());
                }
                "modifier" => {
                    modifiers.push(text.to_string());
                }
                "param_type" => {
                    param_type.push_str(text);
                }
                _ => {}
            }
        }

        if found_param {
            // Build the parameter property line
            let mut param_line = String::new();

            // Check if it's val or var in modifiers, default to val for constructor parameters
            if modifiers.iter().any(|m| m == "var") {
                param_line.push_str("var ");
            } else {
                param_line.push_str("val ");
            }

            param_line.push_str(field_name);
            param_line.push_str(": ");
            param_line.push_str(&param_type);

            let hover = HoverSignature::new("kotlin")
                .with_annotations(annotations)
                .with_modifiers(modifiers)
                .with_signature_line(param_line);

            return Some(hover.format());
        }
    }

    None
}


