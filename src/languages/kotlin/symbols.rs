use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    constants::KOTLIN_PARSER,
    dependency_cache::symbol_index::{ParsedSourceFile, SymbolDefinition},
    symbols::SymbolType,
};

static EXTRACT_SYMBOL_QUERIES: OnceLock<Vec<(Query, SymbolType)>> = OnceLock::new();

#[tracing::instrument(skip_all)]
fn get_extract_symbol_queries() -> &'static [(Query, SymbolType)] {
    EXTRACT_SYMBOL_QUERIES.get_or_init(|| {
        let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
        [
            (
                r#"(class_declaration 
                    (modifiers)?
                    (type_identifier) @name
                )"#,
                SymbolType::ClassDeclaration,
            ),
            (
                r#"(interface_declaration
                    (modifiers)?
                    (type_identifier) @name
                )"#,
                SymbolType::InterfaceDeclaration,
            ),
            (
                r#"(class_declaration
                    (modifiers
                        "enum")
                    (type_identifier) @name
                )"#,
                SymbolType::EnumDeclaration,
            ),
            (
                r#"(class_declaration
                    (modifiers
                        "annotation")
                    (type_identifier) @name
                )"#,
                SymbolType::AnnotationDeclaration,
            ),
            (
                r#"(object_declaration 
                    (modifiers)?
                    (type_identifier) @name
                )"#,
                SymbolType::ClassDeclaration, // Objects are treated as special classes
            ),
            (
                r#"(type_alias
                    (modifiers)?
                    (type_identifier) @name
                )"#,
                SymbolType::Type,
            ),
            (
                r#"(function_declaration
                    (modifiers)?
                    (simple_identifier) @name
                )"#,
                SymbolType::MethodDeclaration,
            ),
            (
                r#"(property_declaration
                    (modifiers)?
                    (binding_pattern_kind)?
                    (variable_declaration
                        (simple_identifier) @name
                    )
                )"#,
                SymbolType::FieldDeclaration,
            ),
        ]
        .iter()
        .filter_map(|(text, sym_type)| {
            match Query::new(language, text) {
                Ok(query) => Some((query, sym_type.clone())),
                Err(e) => {
                    tracing::error!("Failed to create TreeSitter query for {:?}: {} - Query: {}", sym_type, e, text);
                    None
                }
            }
        })
        .collect()
    })
}

#[tracing::instrument(skip_all)]
pub fn extract_kotlin_symbols(parsed_file: &ParsedSourceFile) -> Result<Vec<SymbolDefinition>> {
    let mut symbols = Vec::new();


    let package = extract_kotlin_package(&parsed_file.tree, &parsed_file.content);
    let queries = get_extract_symbol_queries();

    for (query, symbol_type) in queries {
        let mut cursor = QueryCursor::new();

        let mut matches = cursor.matches(
            query,
            parsed_file.tree.root_node(),
            parsed_file.content.as_bytes(),
        );

        while let Some(query_match) = matches.next() {
            let mut symbol_name = None;
            let mut extends = None;
            let mut implements = Vec::new();

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];

                if let Ok(text) = capture.node.utf8_text(parsed_file.content.as_bytes()) {
                    match capture_name {
                        "name" => {
                            symbol_name = Some(text.to_string());
                        },
                        "extends" => extends = Some(text.to_string()),
                        "implements" => implements.push(text.to_string()),
                        _ => {}
                    }
                }
            }

            if let Some(name) = symbol_name {
                // Find the actual name node from the captures
                let name_node = query_match.captures.iter()
                    .find(|capture| {
                        let capture_name = query.capture_names()[capture.index as usize];
                        capture_name == "name"
                    })
                    .map(|capture| capture.node)
                    .unwrap_or_else(|| query_match.captures[0].node);

                // For class and interface declarations, extract inheritance information
                if matches!(symbol_type, SymbolType::ClassDeclaration | SymbolType::InterfaceDeclaration) {
                    let declaration_node = find_declaration_node(&name_node);
                    if let Some(declaration_node) = declaration_node {
                        let inheritance_info = extract_kotlin_inheritance(&declaration_node, &parsed_file.content);
                        extends = inheritance_info.extends;
                        implements = inheritance_info.implements;
                    }
                }

                if is_kotlin_symbol_accessible(&name_node, &parsed_file.content) {
                    let fully_qualified_name = if let Some(ref pkg) = package {
                        format!("{}.{}", pkg, name)
                    } else {
                        continue;
                    };

                    symbols.push(SymbolDefinition {
                        fully_qualified_name,
                        source_file: parsed_file.file_path.clone(),
                        line: name_node.start_position().row,
                        column: name_node.start_position().column,
                        extends,
                        implements,
                    });
                }
            }
        }
    }

    
    Ok(symbols)
}


#[tracing::instrument(skip_all)]
fn extract_kotlin_package(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"(package_header (identifier) @package)"#;
    
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                return Some(text.to_string());
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn is_kotlin_symbol_accessible(name_node: &Node, source: &str) -> bool {
    // Check if the symbol has public or internal visibility
    // Walk up the tree to find modifiers
    let mut current = name_node.parent();
    
    while let Some(node) = current {
        // Look for modifiers node
        for child in node.children(&mut node.walk()) {
            if child.kind() == "modifiers" {
                if let Ok(modifiers_text) = child.utf8_text(source.as_bytes()) {
                    // Private symbols are not accessible from other files
                    if modifiers_text.contains("private") {
                        return false;
                    }
                }
            }
        }
        
        // Check parent declarations that might affect visibility
        match node.kind() {
            "class_declaration" | "object_declaration" | "interface_declaration" => {
                // Check if the containing class/object/interface is private
                for child in node.children(&mut node.walk()) {
                    if child.kind() == "modifiers" {
                        if let Ok(modifiers_text) = child.utf8_text(source.as_bytes()) {
                            if modifiers_text.contains("private") {
                                return false;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        
        current = node.parent();
    }
    
    // Default to accessible (public/internal)
    true
}

#[derive(Debug)]
struct InheritanceInfo {
    extends: Option<String>,
    implements: Vec<String>,
}

/// Find the class_declaration or interface_declaration node that contains the name node
#[tracing::instrument(skip_all)]
fn find_declaration_node<'a>(name_node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*name_node);
    
    while let Some(node) = current {
        if matches!(node.kind(), "class_declaration" | "interface_declaration" | "object_declaration") {
            return Some(node);
        }
        current = node.parent();
    }
    
    None
}

/// Extract inheritance information from a Kotlin class or interface declaration
#[tracing::instrument(skip_all)]
fn extract_kotlin_inheritance(declaration_node: &Node, source: &str) -> InheritanceInfo {
    let mut extends = None;
    let mut implements = Vec::new();
    
    // Look for delegation_specifier children
    for child in declaration_node.children(&mut declaration_node.walk()) {
        if child.kind() == "delegation_specifier" {
            // Check if this is a constructor_invocation (extends) or user_type (implements)
            for delegation_child in child.children(&mut child.walk()) {
                match delegation_child.kind() {
                    "constructor_invocation" => {
                        // This is class inheritance (extends)
                        if let Some(extends_name) = extract_type_name_from_constructor_invocation(&delegation_child, source) {
                            extends = Some(extends_name);
                        }
                    }
                    "user_type" => {
                        // This is interface implementation (implements)
                        if let Some(implements_name) = extract_type_name_from_user_type(&delegation_child, source) {
                            implements.push(implements_name);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    
    InheritanceInfo { extends, implements }
}

/// Extract type name from constructor_invocation node
#[tracing::instrument(skip_all)]
fn extract_type_name_from_constructor_invocation(constructor_node: &Node, source: &str) -> Option<String> {
    for child in constructor_node.children(&mut constructor_node.walk()) {
        if child.kind() == "user_type" {
            return extract_type_name_from_user_type(&child, source);
        }
    }
    None
}

/// Extract type name from user_type node
#[tracing::instrument(skip_all)]
fn extract_type_name_from_user_type(user_type_node: &Node, source: &str) -> Option<String> {
    for child in user_type_node.children(&mut user_type_node.walk()) {
        if child.kind() == "type_identifier" {
            if let Ok(type_name) = child.utf8_text(source.as_bytes()) {
                return Some(type_name.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn parse_kotlin_code(source: &str) -> ParsedSourceFile {
        let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
        let mut parser = Parser::new();
        parser.set_language(language).unwrap();
        let tree = parser.parse(source, None).unwrap();

        ParsedSourceFile {
            file_path: PathBuf::from("test.kt"),
            content: source.to_string(),
            tree,
            language: "kotlin".to_string(),
        }
    }

    #[test]
    fn test_extract_class_symbols() {
        let source = r#"
package com.example

class MyClass : BaseClass, MyInterface {
    fun method() {}
}
"#;
        let parsed_file = parse_kotlin_code(source);
        let symbols = extract_kotlin_symbols(&parsed_file).unwrap();
        
        assert_eq!(symbols.len(), 2); // class + method
        
        let class_symbol = &symbols[0];
        assert_eq!(class_symbol.fully_qualified_name, "com.example.MyClass");
    }

    #[test]
    fn test_extract_object_symbols() {
        let source = r#"
package com.example

object MySingleton {
    fun doSomething() {}
}
"#;
        let parsed_file = parse_kotlin_code(source);
        let symbols = extract_kotlin_symbols(&parsed_file).unwrap();
        
        assert!(!symbols.is_empty());
        let object_symbol = &symbols[0];
        assert_eq!(object_symbol.fully_qualified_name, "com.example.MySingleton");
    }

    #[test]
    fn test_extract_interface_symbols() {
        let source = r#"
package com.example

interface MyInterface {
    fun abstractMethod()
}
"#;
        let parsed_file = parse_kotlin_code(source);
        let symbols = extract_kotlin_symbols(&parsed_file).unwrap();
        
        assert!(!symbols.is_empty());
        // Check that we extracted an interface symbol
        let interface_symbol = symbols.iter()
            .find(|s| s.fully_qualified_name.ends_with("MyInterface"));
        assert!(interface_symbol.is_some());
    }

    #[test]
    fn test_private_symbols_filtered() {
        let source = r#"
package com.example

private class PrivateClass

class PublicClass
"#;
        let parsed_file = parse_kotlin_code(source);
        let symbols = extract_kotlin_symbols(&parsed_file).unwrap();
        
        // Should only contain the public class
        assert!(symbols.iter().any(|s| s.fully_qualified_name.ends_with("PublicClass")));
        // Private class should be filtered out
        assert!(!symbols.iter().any(|s| s.fully_qualified_name.ends_with("PrivateClass")));
    }
}