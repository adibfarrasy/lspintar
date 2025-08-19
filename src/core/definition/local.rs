use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, QueryCursor, StreamingIterator, Tree};

use crate::core::{symbols::SymbolType, utils::node_to_lsp_location};
use crate::languages::traits::LanguageSupport;

use super::scope::{calculate_scope_distance, find_containing_method};

/// Generic local definition finder that works across languages
pub fn find_local_generic(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let definition_node = search_local_definitions_generic(tree, source, usage_node, language_support)?;
    node_to_lsp_location(&definition_node, file_uri)
}

/// Generic local definition search that works across languages
pub fn search_local_definitions_generic<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    language_support: &dyn LanguageSupport,
) -> Option<Node<'a>> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
    
    let symbol_type = language_support
        .determine_symbol_type_from_context(tree, usage_node, source)
        .ok()?;

    // Skip if this is already a declaration
    if symbol_type.is_declaration() {
        return None;
    }

    match symbol_type {
        SymbolType::MethodCall | SymbolType::FunctionCall => {
            // Use specialized method resolution for method calls
            find_best_method_match_generic(tree, source, usage_node, symbol_name, language_support)
        }
        SymbolType::VariableUsage => {
            // Search for variable declarations in accessible scopes
            let variable_candidates = find_variable_declarations_in_scope_generic(
                tree, source, usage_node, symbol_name, language_support
            );
            
            if !variable_candidates.is_empty() {
                // Find the best match using scope distance
                if let Some(best_match) = find_closest_declaration(usage_node, &variable_candidates) {
                    return Some(best_match);
                }
            }
            
            // Not found as variable, try as field
            find_as_field_generic(tree, source, symbol_name, language_support)
        }
        SymbolType::FieldUsage => {
            // For field usage, search using field queries
            let candidates = find_candidates_with_queries(
                tree, source, symbol_name, language_support.field_declaration_queries()
            )?;
            
            // Fields are class-level, so we don't need closest scope logic
            candidates.into_iter().next()
        }
        _ => {
            // For other symbol types, use general candidate finding
            let queries = match symbol_type {
                SymbolType::Type => language_support.class_declaration_queries(),
                _ => &[], // TODO: Add more mappings as needed
            };
            
            if !queries.is_empty() {
                let candidates = find_candidates_with_queries(tree, source, symbol_name, queries)?;
                candidates.into_iter().next()
            } else {
                None
            }
        }
    }
}

/// Find variable declarations that are accessible from the usage point
pub fn find_variable_declarations_in_scope_generic<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
    language_support: &dyn LanguageSupport,
) -> Vec<Node<'a>> {
    let mut candidates = Vec::new();
    
    // 1. Check method parameters first (highest priority)
    if let Some(method) = find_containing_method(usage_node) {
        let param_queries = language_support.parameter_queries();
        for query_text in param_queries {
            if let Some(param_candidates) = find_candidates_with_queries(tree, source, symbol_name, &[query_text]) {
                for candidate in param_candidates {
                    // Only include parameters from the same method
                    if let Some(param_method) = find_containing_method(&candidate) {
                        if param_method.id() == method.id() {
                            candidates.push(candidate);
                        }
                    }
                }
            }
        }
    }
    
    // 2. Walk up from usage node to find variable declarations in accessible scopes
    let mut current_node = Some(*usage_node);
    
    while let Some(node) = current_node {
        // For each ancestor node, check its children for variable declarations
        if matches!(node.kind(), "block" | "method_declaration" | "class_declaration") {
            let var_queries = language_support.variable_declaration_queries();
            
            for query_text in var_queries {
                if let Some(var_candidates) = find_candidates_with_queries(tree, source, symbol_name, &[query_text]) {
                    for var_decl in var_candidates {
                        // Make sure declaration comes before usage
                        if var_decl.start_position() < usage_node.start_position() {
                            // Handle different types of declarations
                            if var_decl.kind() == "identifier" && 
                               var_decl.utf8_text(source.as_bytes()).unwrap_or("") == symbol_name {
                                // For bare identifier declarations
                                candidates.push(var_decl);
                            } else {
                                // Find the actual identifier node within the declaration
                                if let Some(identifier) = find_identifier_in_declaration(&var_decl, source, symbol_name) {
                                    candidates.push(identifier);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        current_node = node.parent();
    }
    
    candidates
}

/// Find the identifier node within a variable declaration that matches the symbol name
fn find_identifier_in_declaration<'a>(
    var_decl: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Option<Node<'a>> {
    let mut cursor = var_decl.walk();
    
    for child in var_decl.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            // Look for the identifier in the variable declarator
            let mut declarator_cursor = child.walk();
            for declarator_child in child.children(&mut declarator_cursor) {
                if declarator_child.kind() == "identifier" {
                    if let Ok(id_text) = declarator_child.utf8_text(source.as_bytes()) {
                        if id_text == symbol_name {
                            return Some(declarator_child);
                        }
                    }
                }
            }
        }
    }
    
    None
}

/// Find the closest declaration from a list of candidates
fn find_closest_declaration<'a>(usage_node: &Node, candidates: &[Node<'a>]) -> Option<Node<'a>> {
    let mut best_candidate = None;
    let mut best_scope_distance = usize::MAX;

    for candidate in candidates.iter() {
        if let Some(distance) = calculate_scope_distance(usage_node, candidate) {
            if distance < best_scope_distance {
                best_scope_distance = distance;
                best_candidate = Some(*candidate);
            }
        }
    }

    best_candidate
}

/// Generic method to find candidates using tree-sitter queries
fn find_candidates_with_queries<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    queries: &[&str],
) -> Option<Vec<Node<'a>>> {
    let mut all_candidates = Vec::new();
    let language = tree.language();
    
    for query_text in queries {
        if let Ok(query) = tree_sitter::Query::new(&language, query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                        if node_text == symbol_name {
                            if let Some(parent) = capture.node.parent() {
                                all_candidates.push(parent);
                            }
                        }
                    }
                }
            }
        }
    }
    
    if all_candidates.is_empty() {
        None
    } else {
        Some(all_candidates)
    }
}

/// Generic method resolution for method calls
fn find_best_method_match_generic<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Node<'a>> {
    let method_queries = language_support.method_declaration_queries();
    let candidates = find_candidates_with_queries(tree, source, symbol_name, method_queries)?;
    
    // TODO: Add signature-based method matching
    // For now, just return the first match
    candidates.into_iter().next()
}

/// Try to find a symbol as a field declaration
fn find_as_field_generic<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Node<'a>> {
    let field_queries = language_support.field_declaration_queries();
    let candidates = find_candidates_with_queries(tree, source, symbol_name, field_queries)?;
    candidates.into_iter().next()
}