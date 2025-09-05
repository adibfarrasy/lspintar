use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    dependency_cache::symbol_index::{ParsedSourceFile, SymbolDefinition},
    symbols::SymbolType,
};

static EXTRACT_SYMBOL_QUERIES: OnceLock<Vec<(Query, SymbolType)>> = OnceLock::new();

// Enhanced to handle nested declarations with proper fully-qualified names
#[tracing::instrument(skip_all)]
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
            // Nested class declarations
            (
                r#"(class_declaration 
                    body: (class_body 
                        (class_declaration 
                            name: (identifier) @name
                            superclass: (superclass (type_identifier) @extends)?
                            interfaces: (super_interfaces (type_list (type_identifier) @implements)*)?))
                )"#,
                SymbolType::ClassDeclaration,
            ),
            // Nested interface declarations
            (
                r#"(class_declaration 
                    body: (class_body 
                        (interface_declaration 
                            name: (identifier) @name
                            interfaces: (extends_interfaces (type_list (type_identifier) @extends)*)?))
                )"#,
                SymbolType::InterfaceDeclaration,
            ),
            // Nested enum declarations
            (
                r#"(class_declaration 
                    body: (class_body 
                        (enum_declaration 
                            name: (identifier) @name
                            interfaces: (super_interfaces (type_list (type_identifier) @implements)*)?))
                )"#,
                SymbolType::EnumDeclaration,
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

#[tracing::instrument(skip_all)]
pub fn extract_groovy_symbols(parsed_file: &ParsedSourceFile) -> Result<Vec<SymbolDefinition>> {
    let mut symbols = Vec::new();

    let package = extract_groovy_package(&parsed_file.tree, &parsed_file.content);
    let queries = get_extract_symbol_queries();

    for (query, _symbol_type) in queries {
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
            let mut name_node = None;

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];

                if let Ok(text) = capture.node.utf8_text(parsed_file.content.as_bytes()) {
                    match capture_name {
                        "name" => {
                            symbol_name = Some(text.to_string());
                            name_node = Some(capture.node);
                        },
                        "extends" => extends = Some(text.to_string()),
                        "implements" => implements.push(text.to_string()),
                        _ => {}
                    }
                }
            }

            if let (Some(name), Some(node)) = (symbol_name, name_node) {
                if is_groovy_symbol_accessible(&node, &parsed_file.content) {
                    // Build proper FQN considering nesting
                    let fully_qualified_name = if let Some(ref pkg) = package {
                        let nested_name = build_nested_class_name(&node, &parsed_file.content, &name);
                        format!("{}.{}", pkg, nested_name)
                    } else {
                        return;
                    };

                    symbols.push(SymbolDefinition {
                        fully_qualified_name,
                        source_file: parsed_file.file_path.clone(),
                        line: node.start_position().row,
                        column: node.start_position().column,
                        extends,
                        implements,
                    });
                }
            }
        });
    }

    Ok(symbols)
}

#[tracing::instrument(skip_all)]
pub fn extract_groovy_package(tree: &Tree, content: &str) -> Option<String> {
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

#[tracing::instrument(skip_all)]
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
        | "function_declaration" => true,
        "field_declaration" => true,
        "property_declaration" => true,
        _ => false,
    }
}

/// Build proper nested class name (e.g., OuterClass$InnerClass)
#[tracing::instrument(skip_all)]
fn build_nested_class_name(node: &Node, content: &str, class_name: &str) -> String {
    let mut class_names = vec![class_name.to_string()];
    let mut current_node = node.parent();

    // Walk up the tree to find parent class declarations
    while let Some(parent) = current_node {
        if parent.kind() == "class_declaration" || 
           parent.kind() == "interface_declaration" || 
           parent.kind() == "enum_declaration" {
            
            // Skip if this is the same class declaration we started from
            // (avoid duplicating the class name for top-level classes)
            if let Some(parent_name_node) = parent.child_by_field_name("name") {
                if let Ok(parent_name) = parent_name_node.utf8_text(content.as_bytes()) {
                    // Only add if it's different from our current class name
                    if parent_name != class_name {
                        class_names.push(parent_name.to_string());
                    }
                }
            }
        }
        current_node = parent.parent();
    }

    // Reverse to get outer-to-inner order and join with $
    class_names.reverse();
    class_names.join("$")
}
