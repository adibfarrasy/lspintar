use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::LanguageSupport,
};

pub fn handle(
    tree: &Tree,
    source: &str,
    location: Location,
    language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    let node = location_to_node(&location, tree)?;

    let symbol_type = language_support
        .determine_symbol_type_from_context(tree, &node, source)
        .ok()?;

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, &node, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, &node, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            // Type could be class, interface, enum, etc. - check the actual node
            match node.kind() {
                "class_declaration" => extract_class_signature(tree, &node, source),
                "interface_declaration" => extract_interface_signature(tree, &node, source),
                "enum_declaration" => extract_enum_signature(tree, &node, source),
                _ => extract_type_usage_info(&node, source),
            }
        },
        SymbolType::MethodCall => {
            // For method calls, try to find the declaration first
            if let Some(method_decl_node) = find_method_declaration_for_call(tree, &node, source) {
                extract_method_signature(tree, &method_decl_node, source)
            } else {
                extract_method_call_info(&node, source)
            }
        },
        SymbolType::VariableDeclaration | SymbolType::VariableUsage => {
            extract_variable_info(tree, &node, source)
        },
        _ => None,
    };

    content.map(|c| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: c,
        }),
        range: Some(location.range),
    })
}

fn extract_class_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Find the class declaration node if we're not already on it
    let class_node = find_parent_of_kind(node, "class_declaration")?;
    
    let mut signature = String::new();
    
    // Extract modifiers
    if let Some(modifiers) = class_node.child_by_field_name("modifiers") {
        if let Ok(mod_text) = modifiers.utf8_text(source.as_bytes()) {
            signature.push_str(mod_text);
            signature.push(' ');
        }
    }
    
    signature.push_str("class ");
    
    // Extract class name
    if let Some(name) = class_node.child_by_field_name("name") {
        if let Ok(name_text) = name.utf8_text(source.as_bytes()) {
            signature.push_str(name_text);
        }
    }
    
    // Extract superclass
    if let Some(superclass) = class_node.child_by_field_name("superclass") {
        if let Ok(super_text) = superclass.utf8_text(source.as_bytes()) {
            signature.push_str(" extends ");
            signature.push_str(super_text.trim_start_matches("extends").trim());
        }
    }
    
    // Extract interfaces
    if let Some(interfaces) = class_node.child_by_field_name("interfaces") {
        if let Ok(int_text) = interfaces.utf8_text(source.as_bytes()) {
            signature.push_str(" implements ");
            signature.push_str(int_text.trim_start_matches("implements").trim());
        }
    }
    
    Some(format!("```java\n{}\n```", signature))
}

fn extract_interface_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let interface_node = find_parent_of_kind(node, "interface_declaration")?;
    
    let mut signature = String::new();
    
    // Extract modifiers
    if let Some(modifiers) = interface_node.child_by_field_name("modifiers") {
        if let Ok(mod_text) = modifiers.utf8_text(source.as_bytes()) {
            signature.push_str(mod_text);
            signature.push(' ');
        }
    }
    
    signature.push_str("interface ");
    
    // Extract interface name
    if let Some(name) = interface_node.child_by_field_name("name") {
        if let Ok(name_text) = name.utf8_text(source.as_bytes()) {
            signature.push_str(name_text);
        }
    }
    
    // Extract extends interfaces
    if let Some(extends) = interface_node.child_by_field_name("extends") {
        if let Ok(ext_text) = extends.utf8_text(source.as_bytes()) {
            signature.push_str(" extends ");
            signature.push_str(ext_text.trim_start_matches("extends").trim());
        }
    }
    
    Some(format!("```java\n{}\n```", signature))
}

fn extract_enum_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let enum_node = find_parent_of_kind(node, "enum_declaration")?;
    
    let mut signature = String::new();
    
    // Extract modifiers
    if let Some(modifiers) = enum_node.child_by_field_name("modifiers") {
        if let Ok(mod_text) = modifiers.utf8_text(source.as_bytes()) {
            signature.push_str(mod_text);
            signature.push(' ');
        }
    }
    
    signature.push_str("enum ");
    
    // Extract enum name
    if let Some(name) = enum_node.child_by_field_name("name") {
        if let Ok(name_text) = name.utf8_text(source.as_bytes()) {
            signature.push_str(name_text);
        }
    }
    
    Some(format!("```java\n{}\n```", signature))
}

fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let method_node = find_parent_of_kind(node, "method_declaration")
        .or_else(|| find_parent_of_kind(node, "constructor_declaration"))?;
    
    if let Ok(method_text) = method_node.utf8_text(source.as_bytes()) {
        // Extract just the method signature (first line typically)
        let signature = method_text.lines().next().unwrap_or(method_text);
        return Some(format!("```java\n{}\n```", signature.trim()));
    }
    
    None
}

fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let field_node = find_parent_of_kind(node, "field_declaration")?;
    
    if let Ok(field_text) = field_node.utf8_text(source.as_bytes()) {
        return Some(format!("```java\n{}\n```", field_text.trim()));
    }
    
    None
}

fn extract_variable_info(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Try to find variable declaration
    let var_node = find_parent_of_kind(node, "variable_declaration")
        .or_else(|| find_parent_of_kind(node, "local_variable_declaration"))?;
    
    if let Ok(var_text) = var_node.utf8_text(source.as_bytes()) {
        return Some(format!("```java\n{}\n```", var_text.trim()));
    }
    
    None
}

fn extract_type_usage_info(node: &Node, source: &str) -> Option<String> {
    if let Ok(type_text) = node.utf8_text(source.as_bytes()) {
        Some(format!("```java\n{}\n```\n\n*Type reference*", type_text))
    } else {
        None
    }
}

fn extract_method_call_info(node: &Node, source: &str) -> Option<String> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;
    
    // Try to find the method invocation parent
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_invocation" {
            if let Ok(call_text) = parent.utf8_text(source.as_bytes()) {
                return Some(format!("```java\n{}\n```\n\n*Method call - definition not found in current file*", 
                                   call_text.trim()));
            }
        }
        current = parent.parent();
    }
    
    Some(format!("```java\n{}\n```\n\n*Method reference*", method_name))
}

fn find_method_declaration_for_call<'a>(tree: &'a Tree, node: &Node, source: &str) -> Option<Node<'a>> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;
    
    let query_text = r#"
        (method_declaration
          name: (identifier) @method_name
        )
    "#;
    
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    let query = Query::new(&language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == method_name {
                    return capture.node.parent();
                }
            }
        }
    }
    
    None
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