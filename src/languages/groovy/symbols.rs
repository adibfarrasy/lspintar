use std::sync::OnceLock;

use anyhow::{Context, Result};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    dependency_cache::symbol_index::{ParsedSourceFile, SymbolDefinition},
    symbols::SymbolType,
};

static EXTRACT_SYMBOL_QUERIES: OnceLock<Vec<(Query, SymbolType)>> = OnceLock::new();

// TODO: currently only handles non-nested declarations
// enhance with recursion to create proper fully-qualified names for inner classes, methods, and
// properties.
fn get_extract_symbol_queries() -> &'static [(Query, SymbolType)] {
    EXTRACT_SYMBOL_QUERIES.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        [
            (
                r#"(class_declaration 
                    name: (identifier) @name
                    superclass: (superclass (type_identifier) @extends)?
                    interfaces: (super_interfaces (type_list (type_identifier) @implements)*)?)
                "#,
                SymbolType::ClassDeclaration,
            ),
            (
                r#"(interface_declaration 
                    name: (identifier) @name
                    interfaces: (extends_interfaces (type_list (type_identifier) @extends)*)?)
                "#,
                SymbolType::InterfaceDeclaration,
            ),
            (
                r#"(enum_declaration 
                    name: (identifier) @name
                    interfaces: (super_interfaces (type_list (type_identifier) @implements)*)?)
                "#,
                SymbolType::EnumDeclaration,
            ),
            (
                r#"(annotation_type_declaration name: (identifier) @name)"#,
                SymbolType::AnnotationDeclaration,
            ),
        ]
        .iter()
        .filter_map(|(text, sym_type)| {
            Query::new(&language, text)
                .ok()
                .map(|q| (q, sym_type.clone()))
        })
        .collect()
    })
}

pub fn extract_groovy_symbols(parsed_file: &ParsedSourceFile) -> Result<Vec<SymbolDefinition>> {
    let mut symbols = Vec::new();

    let package = extract_groovy_package(&parsed_file.tree, &parsed_file.content);
    let queries = get_extract_symbol_queries();

    for (query, symbol_type) in queries {
        let mut cursor = QueryCursor::new();

        let matches = cursor.matches(
            &query,
            parsed_file.tree.root_node(),
            parsed_file.content.as_bytes(),
        );

        matches.for_each(|query_match| {
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
                let name_node = query_match.captures[0].node;

                if is_groovy_symbol_accessible(&name_node, &parsed_file.content) {
                    let fully_qualified_name = if let Some(ref pkg) = package {
                        format!("{}.{}", pkg, name)
                    } else {
                        return;
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
        });
    }

    Ok(symbols)
}

fn extract_groovy_package(tree: &Tree, content: &str) -> Option<String> {
    let query_text = r#"(package_declaration (scoped_identifier) @package)"#;
    let query = Query::new(&tree_sitter_groovy::language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut result = None;
    cursor
        .matches(&query, tree.root_node(), content.as_bytes())
        .take(1)
        .for_each(|query_match| {
            for capture in query_match.captures {
                result = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .ok()
                    .map(String::from);
            }
        });

    result
}

fn is_groovy_symbol_accessible(node: &Node, content: &str) -> bool {
    let mut declaration_node = node.parent();
    while let Some(parent) = declaration_node {
        if parent.kind().ends_with("_declaration") {
            break;
        }
        declaration_node = parent.parent();
    }

    let Some(decl) = declaration_node else {
        return false; // Can't find declaration
    };

    for child in decl.children(&mut decl.walk()) {
        if child.kind() == "modifiers" {
            let modifier_text = child.utf8_text(content.as_bytes()).unwrap_or("");

            if modifier_text.contains("private") {
                return false;
            }

            if modifier_text.contains("public") {
                return true;
            }

            // Accessible for inheritance
            if modifier_text.contains("protected") {
                return true;
            }
        }
    }

    // Groovy default visibility rules:
    // - Classes, interfaces, enums, methods: public by default
    // - Fields: private by default, but indexed anyway
    match decl.kind() {
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "method_declaration" => true,
        "field_declaration" => true,
        "property_declaration" => true,
        _ => false,
    }
}
