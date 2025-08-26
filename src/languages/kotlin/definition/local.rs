use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree, Query, QueryCursor, StreamingIterator};

use crate::{
    core::{utils::node_to_lsp_location, constants::KOTLIN_PARSER},
    languages::LanguageSupport,
};

pub fn find_local(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Search for declarations in the same file using tree-sitter queries
    if let Some(definition_node) = search_local_definitions_with_queries(tree, source, usage_node, &symbol_name) {
        return node_to_lsp_location(&definition_node, file_uri);
    }

    // Fallback to simple traversal search
    if let Some(definition_node) = search_local_definitions(tree, source, usage_node, &symbol_name) {
        return node_to_lsp_location(&definition_node, file_uri);
    }

    None
}

/// Search for local definitions using tree-sitter queries for better accuracy
fn search_local_definitions_with_queries<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node,
    symbol_name: &str,
) -> Option<Node<'a>> {
    let usage_byte_offset = usage_node.start_byte();
    
    // Define comprehensive queries for Kotlin declarations
    let queries = [
        // Variable declarations
        r#"(property_declaration (variable_declaration (simple_identifier) @name))"#,
        // Function declarations
        r#"(function_declaration (simple_identifier) @name)"#,
        // Class declarations
        r#"(class_declaration (type_identifier) @name)"#,
        // Interface declarations
        r#"(interface_declaration (type_identifier) @name)"#,
        // Object declarations
        r#"(object_declaration (type_identifier) @name)"#,
        // Parameters in functions
        r#"(function_value_parameters (parameter (simple_identifier) @name))"#,
        r#"(primary_constructor (class_parameters (class_parameter (simple_identifier) @name)))"#,
        // Lambda parameters
        r#"(lambda_parameters (lambda_parameter (simple_identifier) @name))"#,
        // For loop variables
        r#"(for_statement (multi_variable_declaration (variable_declaration (simple_identifier) @name)))"#,
        r#"(for_statement (variable_declaration (simple_identifier) @name))"#,
        // Catch parameters
        r#"(catch_block (simple_identifier) @name)"#,
    ];

    for query_text in &queries {
        if let Some(node) = execute_query_for_symbol(tree, source, query_text, symbol_name, usage_byte_offset) {
            return Some(node);
        }
    }

    None
}

fn execute_query_for_symbol<'a>(
    tree: &'a Tree,
    source: &str,
    query_text: &str,
    symbol_name: &str,
    usage_byte_offset: usize,
) -> Option<Node<'a>> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == symbol_name && capture.node.start_byte() < usage_byte_offset {
                    // Return the declaration node, not just the identifier
                    return find_declaration_parent(capture.node);
                }
            }
        }
    }
    
    None
}

fn find_declaration_parent(identifier_node: Node) -> Option<Node> {
    let mut current = identifier_node;
    
    // Traverse up to find the actual declaration node
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "property_declaration" | "function_declaration" | "class_declaration" 
            | "object_declaration" | "parameter" | "class_parameter" 
            | "lambda_parameter" | "catch_block" => {
                return Some(parent);
            }
            _ => current = parent,
        }
    }
    
    Some(identifier_node)
}

fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node,
    symbol_name: &str,
) -> Option<Node<'a>> {
    // Simple approach: search for matching declarations in the same file
    search_for_declaration(tree.root_node(), source, symbol_name, usage_node.start_byte())
}

fn search_for_declaration<'a>(
    node: Node<'a>,
    source: &str,
    symbol_name: &str,
    usage_byte_offset: usize,
) -> Option<Node<'a>> {
    // Check if this node is a declaration that matches our symbol
    if is_declaration_node(&node) {
        if let Some(declared_name) = get_declared_name(&node, source) {
            if declared_name == symbol_name && node.start_byte() < usage_byte_offset {
                return Some(node);
            }
        }
    }

    // Search children
    for child in node.children(&mut node.walk()) {
        if let Some(result) = search_for_declaration(child, source, symbol_name, usage_byte_offset) {
            return Some(result);
        }
    }

    None
}

fn is_declaration_node(node: &Node) -> bool {
    matches!(
        node.kind(),
        "function_declaration" | "property_declaration" | "class_declaration" 
        | "interface_declaration" | "object_declaration" | "parameter" | "class_parameter" | "lambda_parameter"
        | "catch_block" | "multi_variable_declaration" | "variable_declaration"
        | "enum_declaration" | "annotation_declaration" | "type_alias"
    )
}

fn get_declared_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declaration" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "class_declaration" | "object_declaration" | "interface_declaration" 
        | "enum_declaration" | "annotation_declaration" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "type_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "property_declaration" | "variable_declaration" => {
            // Look for simple_identifier directly or in variable_declaration child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                } else if child.kind() == "variable_declaration" {
                    for grandchild in child.children(&mut child.walk()) {
                        if grandchild.kind() == "simple_identifier" {
                            return grandchild.utf8_text(source.as_bytes()).ok().map(String::from);
                        }
                    }
                }
            }
        }
        "parameter" | "class_parameter" | "lambda_parameter" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "catch_block" => {
            // Catch parameter is directly a simple_identifier
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "type_alias" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "type_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "multi_variable_declaration" => {
            // Look for variable_declaration children
            for child in node.children(&mut node.walk()) {
                if child.kind() == "variable_declaration" {
                    for grandchild in child.children(&mut child.walk()) {
                        if grandchild.kind() == "simple_identifier" {
                            return grandchild.utf8_text(source.as_bytes()).ok().map(String::from);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    None
}