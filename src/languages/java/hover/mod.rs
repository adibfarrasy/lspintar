use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use method::extract_method_signature;
use log::debug;
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::LanguageSupport,
};

mod class;
mod field;
mod interface;
mod method;
mod utils;

pub fn handle(
    tree: &Tree,
    source: &str,
    location: Location,
    language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    debug!("hover::handle: called with location {:?}", location);
    let node = location_to_node(&location, tree);
    if node.is_none() {
        debug!("hover::handle: location_to_node returned None for location {:?}", location);
        return None;
    }
    let node = node?;

    debug!("hover::handle: successfully got node '{}', determining symbol type", node.kind());
    let symbol_type = language_support
        .determine_symbol_type_from_context(tree, &node, source);
    if symbol_type.is_err() {
        debug!("hover::handle: determine_symbol_type_from_context failed: {:?}", symbol_type);
        return None;
    }
    let symbol_type = symbol_type.ok()?;
    debug!("hover::handle: detected symbol_type: {:?}", symbol_type);

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            debug!("hover::handle: handling Type symbol, determining specific type from node");
            // Type could be class, interface, enum, etc. - need to check the actual node
            match node.kind() {
                "class_declaration" => {
                    debug!("hover::handle: Type is class_declaration");
                    extract_class_signature(tree, source)
                },
                "interface_declaration" => {
                    debug!("hover::handle: Type is interface_declaration");
                    extract_interface_signature(tree, source)
                },
                "enum_declaration" => {
                    debug!("hover::handle: Type is enum_declaration");
                    extract_enum_signature(tree, source)
                },
                _ => {
                    debug!("hover::handle: Type has unknown node kind '{}', trying interface extraction first", node.kind());
                    // Try interface extraction first, then fall back to generic type info
                    extract_interface_signature(tree, source)
                        .or_else(|| extract_class_signature(tree, source))
                        .or_else(|| extract_type_usage_info(&node, source))
                }
            }
        },
        SymbolType::MethodCall => {
            // For method calls, try to find the declaration first, then extract signature
            if let Some(method_decl_node) = find_method_declaration_for_call(tree, &node, source) {
                extract_method_signature(tree, &method_decl_node, source)
            } else {
                // Fallback: provide basic method call info
                extract_method_call_info(&node, source)
            }
        },
        SymbolType::VariableDeclaration | SymbolType::VariableUsage => {
            extract_variable_info(tree, &node, source)
        },
        _ => {
            debug!("hover::handle: unsupported symbol type: {:?}", symbol_type);
            None
        },
    };

    if content.is_none() {
        debug!("hover::handle: content extraction returned None for symbol type {:?}", symbol_type);
    }

    content.and_then(|c| {
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: c,
            }),
            range: Some(location.range),
        })
    })
}

fn extract_enum_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (
      (block_comment)? @javadoc
      (enum_declaration
        (modifiers)? @modifiers
        name: (identifier) @enum_name
        interfaces: (super_interfaces)? @interface_line
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse enum query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut enum_name = String::new();
    let mut interface_line = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    // Process all matches but avoid duplicate concatenation
    let mut found_enum = false;
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                },
                "modifiers" => {
                    if modifiers.is_empty() && !found_enum {
                        modifiers.push_str(text);
                    }
                },
                "enum_name" => {
                    if enum_name.is_empty() && !found_enum {
                        enum_name.push_str(text);
                        found_enum = true;
                    }
                },
                "interface_line" => {
                    if interface_line.is_empty() && !found_enum {
                        interface_line = text.to_string();
                    }
                },
                "javadoc" => {
                    if javadoc.is_empty() && !found_enum {
                        javadoc = text.to_string();
                    }
                },
                _ => {}
            }
        }
    }

    format_enum_signature(package_name, modifiers, enum_name, interface_line, javadoc)
}

fn format_enum_signature(
    package_name: String,
    modifiers: String,
    enum_name: String,
    interface_line: String,
    javadoc: String,
) -> Option<String> {
    if enum_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```java".to_string());

    let mut enum_line = String::new();
    
    if !modifiers.is_empty() {
        enum_line.push_str(&modifiers);
        enum_line.push(' ');
    }
    
    enum_line.push_str("enum ");
    enum_line.push_str(&enum_name);
    
    parts.push(enum_line);

    // Add implements clause on separate line
    if !interface_line.is_empty() {
        parts.push(format!("    {}", interface_line));
    }

    parts.push("```".to_string());

    if !javadoc.is_empty() {
        parts.push("\n".to_string());
        parts.push("---".to_string());
        parts.push(javadoc);
    }

    Some(parts.join("\n"))
}

fn extract_type_usage_info(node: &Node, source: &str) -> Option<String> {
    if let Ok(type_text) = node.utf8_text(source.as_bytes()) {
        Some(format!("```java\n{}\n```\n\n*Type reference*", type_text))
    } else {
        None
    }
}

fn extract_variable_info(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Try to find variable declaration
    let var_node = find_parent_of_kind(node, "variable_declaration")
        .or_else(|| find_parent_of_kind(node, "local_variable_declaration"));
    
    if let Some(var_node) = var_node {
        if let Ok(var_text) = var_node.utf8_text(source.as_bytes()) {
            return Some(format!("```java\n{}\n```", var_text.trim()));
        }
    }
    
    None
}

/// Find method declaration for a method call within the same file
fn find_method_declaration_for_call<'a>(tree: &'a Tree, node: &Node, source: &str) -> Option<Node<'a>> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;
    
    let query_text = r#"
        (method_declaration
          name: (identifier) @method_name
        )
    "#;
    
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == method_name {
                    // Return the method declaration node (parent of identifier)
                    return capture.node.parent();
                }
            }
        }
    }
    
    None
}

/// Provide basic method call information when declaration can't be found
fn extract_method_call_info(node: &Node, source: &str) -> Option<String> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;
    
    // Try to find the method invocation parent to get call context
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_invocation" {
            // Extract object and arguments if available
            let object_text = parent.child_by_field_name("object")
                .and_then(|obj| obj.utf8_text(source.as_bytes()).ok())
                .unwrap_or("");
            
            let args_text = parent.child_by_field_name("arguments")
                .and_then(|args| args.utf8_text(source.as_bytes()).ok())
                .unwrap_or("()");
            
            let call_info = if !object_text.is_empty() {
                format!("```java\n{}.{}{}\n```\n\n*Method call - definition not found in current file*", 
                       object_text, method_name, args_text)
            } else {
                format!("```java\n{}{}\n```\n\n*Method call - definition not found in current file*", 
                       method_name, args_text)
            };
            
            return Some(call_info);
        }
        current = parent.parent();
    }
    
    // Fallback for standalone method name
    Some(format!("```java\n{}\n```\n\n*Method reference*", method_name))
}

fn find_parent_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(*node);
    }
    
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return Some(parent);
        }
        current = parent.parent();
    }
    
    None
}