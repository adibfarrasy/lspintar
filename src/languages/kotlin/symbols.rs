use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    constants::KOTLIN_PARSER,
    dependency_cache::symbol_index::{ParsedSourceFile, SymbolDefinition},
    symbols::SymbolType,
};

static EXTRACT_SYMBOL_QUERIES: OnceLock<Vec<(Query, SymbolType)>> = OnceLock::new();

fn get_extract_symbol_queries() -> &'static [(Query, SymbolType)] {
    EXTRACT_SYMBOL_QUERIES.get_or_init(|| {
        let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
        [
            (
                r#"(class_declaration 
                    name: (type_identifier) @name
                    supertype_list: (delegation_specifiers
                        (delegation_specifier (user_type (type_identifier) @extends))*
                        (delegation_specifier (user_type (type_identifier) @implements))*
                    )?
                )"#,
                SymbolType::ClassDeclaration,
            ),
            (
                r#"(interface_declaration
                    (type_identifier) @name
                )"#,
                SymbolType::InterfaceDeclaration,
            ),
            (
                r#"(class_declaration
                    (modifiers "enum")
                    name: (type_identifier) @name
                    supertype_list: (delegation_specifiers
                        (delegation_specifier (user_type (type_identifier) @implements))*
                    )?
                )"#,
                SymbolType::EnumDeclaration,
            ),
            (
                r#"(class_declaration
                    (modifiers "annotation")
                    name: (type_identifier) @name
                )"#,
                SymbolType::AnnotationDeclaration,
            ),
            (
                r#"(object_declaration 
                    name: (type_identifier) @name
                    supertype_list: (delegation_specifiers
                        (delegation_specifier (user_type (type_identifier) @implements))*
                    )?
                )"#,
                SymbolType::ClassDeclaration, // Objects are treated as special classes
            ),
            (
                r#"(type_alias
                    name: (type_identifier) @name
                )"#,
                SymbolType::Type,
            ),
            (
                r#"(function_declaration
                    name: (simple_identifier) @name
                )"#,
                SymbolType::MethodDeclaration,
            ),
            (
                r#"(property_declaration
                    (variable_declaration
                        name: (simple_identifier) @name
                    )
                )"#,
                SymbolType::FieldDeclaration,
            ),
        ]
        .iter()
        .filter_map(|(text, sym_type)| {
            Query::new(language, text)
                .ok()
                .map(|q| (q, sym_type.clone()))
        })
        .collect()
    })
}

pub fn extract_kotlin_symbols(parsed_file: &ParsedSourceFile) -> Result<Vec<SymbolDefinition>> {
    let mut symbols = Vec::new();

    // Add debug logging to track file processing
    tracing::debug!("extract_kotlin_symbols: processing file {:?}", parsed_file.file_path);

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
                        "name" => symbol_name = Some(text.to_string()),
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


                if is_kotlin_symbol_accessible(&name_node, &parsed_file.content) {
                    let fully_qualified_name = if let Some(ref pkg) = package {
                        format!("{}.{}", pkg, name)
                    } else {
                        continue;
                    };

                    symbols.push(SymbolDefinition {
                        name,
                        fully_qualified_name,
                        symbol_type: symbol_type.clone(),
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
        assert_eq!(class_symbol.name, "MyClass");
        assert_eq!(class_symbol.fully_qualified_name, "com.example.MyClass");
        assert_eq!(class_symbol.symbol_type, SymbolType::ClassDeclaration);
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
        assert_eq!(object_symbol.name, "MySingleton");
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
        let interface_symbol = symbols.iter()
            .find(|s| s.symbol_type == SymbolType::InterfaceDeclaration);
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
        assert!(symbols.iter().any(|s| s.name == "PublicClass"));
        // Private class should be filtered out
        assert!(!symbols.iter().any(|s| s.name == "PrivateClass"));
    }
}