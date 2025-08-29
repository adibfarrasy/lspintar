use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use method::extract_method_signature;
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::LanguageSupport,
};

pub mod class;
pub mod field;
pub mod interface;
pub mod method;
pub mod utils;

pub fn handle(
    tree: &Tree,
    source: &str,
    location: Location,
    language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    let node = location_to_node(&location, tree)?;

    let symbol_type = language_support.determine_symbol_type_from_context(tree, &node, source).ok()?;

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            // Type could be class, interface, enum, object, etc. - need to check the actual node
            match node.kind() {
                "class_declaration" if is_enum_class(&node, source) => extract_enum_signature(tree, source),
                "class_declaration" => extract_class_signature(tree, source),
                "interface_declaration" => extract_interface_signature(tree, source),
                "object_declaration" => extract_object_signature(tree, source),
                "type_alias" => extract_type_alias_signature(tree, source),
                _ => {
                    // Try interface extraction first, then fall back to generic type info
                    extract_interface_signature(tree, source)
                        .or_else(|| extract_class_signature(tree, source))
                        .or_else(|| extract_type_usage_info(&node, source))
                }
            }
        }
        SymbolType::MethodCall => {
            // For method calls, try to find the declaration first, then extract signature
            if let Some(method_decl_node) = find_method_declaration_for_call(tree, &node, source) {
                extract_method_signature(tree, &method_decl_node, source)
            } else {
                // Fallback: provide basic method call info
                extract_method_call_info(&node, source)
            }
        }
        SymbolType::VariableDeclaration | SymbolType::VariableUsage => {
            extract_variable_info(tree, &node, source)
        }
        SymbolType::FieldUsage => extract_field_signature(tree, &node, source),
        _ => {
            // Debug unknown symbol types but return None
            tracing::debug!("Kotlin hover: unsupported symbol type: {:?}", symbol_type);
            return None;
        }
    }?;

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: Some(location.range),
    })
}

fn extract_object_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (object_declaration
        (modifiers)? @modifiers
        (type_identifier) @object_name
        (delegation_specifier)* @supertypes
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut object_name = String::new();
    let mut modifiers = String::new();
    let mut supertypes = String::new();

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
                "modifiers" => {
                    if modifiers.is_empty() {
                        modifiers.push_str(text);
                    }
                }
                "object_name" => {
                    if object_name.is_empty() {
                        object_name.push_str(text);
                    }
                }
                "supertypes" => {
                    if supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if object_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    // Package name at the top
    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```kotlin".to_string());

    // Build the object declaration line
    let mut object_line = String::new();

    if !modifiers.is_empty() {
        object_line.push_str(&modifiers);
        object_line.push(' ');
    }

    object_line.push_str("object ");
    object_line.push_str(&object_name);

    parts.push(object_line);

    // Add inheritance line starting with ':' if there are supertypes
    if !supertypes.is_empty() {
        let inheritance_line = format!(": {}", supertypes.replace('\n', ", "));
        parts.push(inheritance_line);
    }

    parts.push("```".to_string());

    Some(parts.join("\n"))
}

fn extract_enum_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (class_declaration
        (modifiers)? @modifiers
        (type_identifier) @enum_name
        (delegation_specifier)* @supertypes
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut enum_name = String::new();
    let mut modifiers = String::new();
    let mut supertypes = String::new();

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
                "modifiers" => {
                    if modifiers.is_empty() {
                        modifiers.push_str(text);
                    }
                }
                "enum_name" => {
                    if enum_name.is_empty() {
                        enum_name.push_str(text);
                    }
                }
                "supertypes" => {
                    if supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if enum_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    // Package name at the top
    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```kotlin".to_string());

    // Build the enum class declaration line
    let mut enum_line = String::new();

    if !modifiers.is_empty() {
        enum_line.push_str(&modifiers);
        enum_line.push(' ');
    }

    enum_line.push_str("enum class ");
    enum_line.push_str(&enum_name);

    parts.push(enum_line);

    // Add inheritance line starting with ':' if there are supertypes
    if !supertypes.is_empty() {
        let inheritance_line = format!(": {}", supertypes.replace('\n', ", "));
        parts.push(inheritance_line);
    }

    parts.push("```".to_string());

    Some(parts.join("\n"))
}

fn extract_type_alias_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (type_alias
        (modifiers)? @modifiers
        (type_identifier) @alias_name
        "=" 
        (_) @target_type
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut alias_name = String::new();
    let mut modifiers = String::new();
    let mut target_type = String::new();

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
                "modifiers" => {
                    if modifiers.is_empty() {
                        modifiers.push_str(text);
                    }
                }
                "alias_name" => {
                    if alias_name.is_empty() {
                        alias_name.push_str(text);
                    }
                }
                "target_type" => {
                    if target_type.is_empty() {
                        target_type.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if alias_name.is_empty() || target_type.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    // Package name at the top
    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```kotlin".to_string());

    // Build the typealias declaration line
    let mut alias_line = String::new();

    if !modifiers.is_empty() {
        alias_line.push_str(&modifiers);
        alias_line.push(' ');
    }

    alias_line.push_str("typealias ");
    alias_line.push_str(&alias_name);
    alias_line.push_str(" = ");
    alias_line.push_str(&target_type.replace('\n', " "));

    parts.push(alias_line);
    parts.push("```".to_string());

    Some(parts.join("\n"))
}

fn extract_type_usage_info(node: &Node, source: &str) -> Option<String> {
    let type_text = node.utf8_text(source.as_bytes()).ok()?;
    Some(format!("```kotlin\n{}\n```", type_text))
}

fn find_method_declaration_for_call<'a>(tree: &'a Tree, node: &'a Node<'a>, source: &str) -> Option<Node<'a>> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;
    
    // Look for function declarations with the same name in the current file
    let query_text = r#"
        (function_declaration
          (simple_identifier) @method_name
        )
    "#;
    
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == method_name {
                    // Find the function_declaration parent node
                    let mut current = Some(capture.node);
                    while let Some(node) = current {
                        if node.kind() == "function_declaration" {
                            return Some(node);
                        }
                        current = node.parent();
                    }
                }
            }
        }
    }
    
    None
}

fn extract_method_call_info(node: &Node, source: &str) -> Option<String> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    // Try to find the call expression parent to get call context
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "call_expression" => {
                // Extract receiver and arguments if available
                let mut receiver_text = String::new();
                let mut args_text = String::from("()");
                
                for child in parent.children(&mut parent.walk()) {
                    match child.kind() {
                        "navigation_expression" => {
                            // Get the receiver (object.method)
                            if let Ok(nav_text) = child.utf8_text(source.as_bytes()) {
                                // Extract just the receiver part (before the last dot)
                                if let Some(dot_pos) = nav_text.rfind('.') {
                                    receiver_text = nav_text[..dot_pos].to_string();
                                }
                            }
                        }
                        "call_suffix" => {
                            // Get the arguments
                            if let Ok(suffix_text) = child.utf8_text(source.as_bytes()) {
                                args_text = suffix_text.to_string();
                            }
                        }
                        _ => {}
                    }
                }

                let call_info = if !receiver_text.is_empty() {
                    format!(
                        "```kotlin\n{}.{}{}\n```\n\n*Method call - definition not found in current file*", 
                        receiver_text, method_name, args_text
                    )
                } else {
                    format!(
                        "```kotlin\n{}{}\n```\n\n*Method call - definition not found in current file*",
                        method_name, args_text
                    )
                };

                return Some(call_info);
            }
            "navigation_expression" => {
                // For obj.method() style calls
                if let Ok(nav_text) = parent.utf8_text(source.as_bytes()) {
                    return Some(format!(
                        "```kotlin\n{}\n```\n\n*Method call - definition not found in current file*",
                        nav_text
                    ));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    // Fallback for standalone method name  
    Some(format!(
        "```kotlin\n{}\n```\n\n*Method reference*",
        method_name
    ))
}

fn extract_variable_info(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Look for the variable declaration
    let query_text = r#"
    (property_declaration
        (modifiers)? @modifiers
        (variable_declaration
            (simple_identifier) @var_name
            (user_type)? @var_type
        )
        ("=" (_))? @initializer
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let var_name = node.utf8_text(source.as_bytes()).ok()?;
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            if capture_name == "var_name" && text == var_name {
                // Found the variable declaration
                let mut signature = String::new();
                signature.push_str("```kotlin\n");

                // Get modifiers
                for capture2 in query_match.captures {
                    let capture2_name = query.capture_names()[capture2.index as usize];
                    let text2 = capture2.node.utf8_text(source.as_bytes()).unwrap_or("");
                    
                    if capture2_name == "modifiers" {
                        signature.push_str(text2);
                        signature.push(' ');
                        break;
                    }
                }

                signature.push_str("val ");
                signature.push_str(var_name);

                // Add type if available
                for capture2 in query_match.captures {
                    let capture2_name = query.capture_names()[capture2.index as usize];
                    let text2 = capture2.node.utf8_text(source.as_bytes()).unwrap_or("");
                    
                    if capture2_name == "var_type" {
                        signature.push_str(": ");
                        signature.push_str(text2);
                        break;
                    }
                }

                signature.push_str("\n```");
                return Some(signature);
            }
        }
    }

    // Fallback: just show the variable name
    Some(format!("```kotlin\nval {}\n```", var_name))
}

/// Check if a class_declaration node is actually an enum class
fn is_enum_class(node: &Node, source: &str) -> bool {
    // Look for the "enum" keyword in modifiers or before "class"
    for child in node.children(&mut node.walk()) {
        if child.kind() == "modifiers" {
            if let Ok(modifiers_text) = child.utf8_text(source.as_bytes()) {
                if modifiers_text.contains("enum") {
                    return true;
                }
            }
        }
    }
    
    // Check if there's an "enum" keyword before the class declaration  
    if let Ok(node_text) = node.utf8_text(source.as_bytes()) {
        node_text.trim_start().starts_with("enum class")
    } else {
        false
    }
}