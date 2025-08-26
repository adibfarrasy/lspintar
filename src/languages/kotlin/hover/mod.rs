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
                "class_declaration" => extract_class_signature(tree, source),
                "interface_declaration" => extract_interface_signature(tree, source),
                "object_declaration" => extract_object_signature(tree, source),
                "enum_declaration" => extract_enum_signature(tree, source),
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
        _ => return None,
    }?;

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn extract_object_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (object_declaration
        (modifiers)? @modifiers
        name: (type_identifier) @object_name
        supertype_list: (delegation_specifiers)? @supertypes
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

    let mut signature = String::new();
    signature.push_str("```kotlin\n");

    if !modifiers.is_empty() {
        signature.push_str(&modifiers);
        signature.push(' ');
    }

    signature.push_str("object ");
    signature.push_str(&object_name);

    if !supertypes.is_empty() {
        signature.push_str(" : ");
        signature.push_str(&supertypes.replace('\n', " "));
    }

    signature.push_str("\n```");

    if !package_name.is_empty() {
        signature.push_str(&format!("\n\n**Package:** `{}`", package_name));
    }

    Some(signature)
}

fn extract_enum_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (enum_declaration
        (modifiers)? @modifiers
        name: (type_identifier) @enum_name
        supertype_list: (delegation_specifiers)? @supertypes
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

    let mut signature = String::new();
    signature.push_str("```kotlin\n");

    if !modifiers.is_empty() {
        signature.push_str(&modifiers);
        signature.push(' ');
    }

    signature.push_str("enum class ");
    signature.push_str(&enum_name);

    if !supertypes.is_empty() {
        signature.push_str(" : ");
        signature.push_str(&supertypes.replace('\n', " "));
    }

    signature.push_str("\n```");

    if !package_name.is_empty() {
        signature.push_str(&format!("\n\n**Package:** `{}`", package_name));
    }

    Some(signature)
}

fn extract_type_alias_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (type_alias
        (modifiers)? @modifiers
        name: (type_identifier) @alias_name
        "=" 
        type: (_) @target_type
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

    let mut signature = String::new();
    signature.push_str("```kotlin\n");

    if !modifiers.is_empty() {
        signature.push_str(&modifiers);
        signature.push(' ');
    }

    signature.push_str("typealias ");
    signature.push_str(&alias_name);
    signature.push_str(" = ");
    signature.push_str(&target_type.replace('\n', " "));

    signature.push_str("\n```");

    if !package_name.is_empty() {
        signature.push_str(&format!("\n\n**Package:** `{}`", package_name));
    }

    Some(signature)
}

fn extract_type_usage_info(node: &Node, source: &str) -> Option<String> {
    let type_text = node.utf8_text(source.as_bytes()).ok()?;
    Some(format!("```kotlin\n{}\n```", type_text))
}

fn find_method_declaration_for_call<'a>(_tree: &'a Tree, _node: &'a Node<'a>, _source: &str) -> Option<Node<'a>> {
    // TODO: Implement method declaration finding for method calls
    // This would involve looking up the method in the current scope or through imports
    None
}

fn extract_method_call_info(node: &Node, source: &str) -> Option<String> {
    let call_text = node.utf8_text(source.as_bytes()).ok()?;
    Some(format!("```kotlin\n{}\n```\n\n*Method call*", call_text))
}

fn extract_variable_info(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Look for the variable declaration
    let query_text = r#"
    (property_declaration
        (modifiers)? @modifiers
        (variable_declaration
            name: (simple_identifier) @var_name
            type: (user_type)? @var_type
        )
        ("=" (expression))? @initializer
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