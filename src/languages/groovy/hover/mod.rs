use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use log::debug;
use method::extract_method_signature;
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
    let node = location_to_node(&location, tree);
    if node.is_none() {
        return None;
    }
    let node = node?;

    let symbol_type = language_support.determine_symbol_type_from_context(tree, &node, source);
    if symbol_type.is_err() {
        return None;
    }
    let symbol_type = symbol_type.ok()?;

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            match node.kind() {
                "class_declaration" => extract_class_signature(tree, source),
                "interface_declaration" => extract_interface_signature(tree, source),
                "enum_declaration" => {
                    // We don't have enum extraction yet, fall back to class
                    extract_class_signature(tree, source)
                }
                _ => extract_class_signature(tree, source)
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
        _ => None
    };


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

/// Find method declaration for a method call within the same file
fn find_method_declaration_for_call<'a>(
    tree: &'a Tree,
    node: &Node,
    source: &str,
) -> Option<Node<'a>> {
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
            let object_text = parent
                .child_by_field_name("object")
                .and_then(|obj| obj.utf8_text(source.as_bytes()).ok())
                .unwrap_or("");

            let args_text = parent
                .child_by_field_name("arguments")
                .and_then(|args| args.utf8_text(source.as_bytes()).ok())
                .unwrap_or("()");

            let call_info = if !object_text.is_empty() {
                format!("```groovy\n{}.{}{}\n```\n\n*Method call - definition not found in current file*", 
                       object_text, method_name, args_text)
            } else {
                format!(
                    "```groovy\n{}{}\n```\n\n*Method call - definition not found in current file*",
                    method_name, args_text
                )
            };

            return Some(call_info);
        }
        current = parent.parent();
    }

    // Fallback for standalone method name
    Some(format!(
        "```groovy\n{}\n```\n\n*Method reference*",
        method_name
    ))
}
