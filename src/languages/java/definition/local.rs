
use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::node_to_lsp_location},
    languages::LanguageSupport,
};

use super::utils::{
    find_definition_candidates, get_declaration_query_for_symbol_type, get_or_create_query,
};

#[tracing::instrument(skip_all)]
pub fn find_local(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let definition_node = search_local_definitions(tree, source, usage_node, language_support)?;

    node_to_lsp_location(&definition_node, file_uri)
}

#[tracing::instrument(skip_all)]
pub fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    language_support: &dyn LanguageSupport,
) -> Option<Node<'a>> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    let symbol_type = language_support
        .determine_symbol_type_from_context(tree, usage_node, source)
        .ok()?;

    if symbol_type.is_declaration() {
        return None;
    }

    match symbol_type {
        SymbolType::MethodCall => {
            // Use specialized method resolution for method calls
            find_best_method_match(tree, source, usage_node, symbol_name)
        }
        SymbolType::VariableUsage => {
            // Search for variable declarations in accessible scopes
            let variable_candidates = find_variable_declarations_in_scope(tree, source, usage_node, &symbol_name);
            
            if !variable_candidates.is_empty() {
                // Find the best match using scope distance
                if let Some(best_match) = find_closest_declaration(usage_node, &variable_candidates) {
                    return Some(best_match);
                }
            }
            
            // Not found as variable, try as field
            find_as_field(tree, source, symbol_name)
        }

        SymbolType::FieldUsage => {
            // Get candidates for field usage
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;
            
            // For field usage, find the field declaration
            // Fields are class-level, so we don't need closest scope logic
            candidates.into_iter().next()
        }
        
        _ => {
            // For other symbol types, use the general candidate finding approach
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;
            candidates.into_iter().next()
        }
    }
}

/// Find variable declarations that are accessible from the usage point
/// This includes:
/// 1. Variable declarations in the same block that come before usage
/// 2. Variable declarations in parent blocks
/// 3. Method parameters
fn find_variable_declarations_in_scope<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Vec<Node<'a>> {
    let mut candidates = Vec::new();
    
    // 1. Check method parameters first (highest priority)
    if let Some(method) = find_containing_method(usage_node) {
        let param_query = r#"(formal_parameter (identifier) @name)"#;
        if let Ok(query) = get_or_create_query(param_query) {
            let mut cursor = QueryCursor::new();
            cursor
                .matches(&query, method, source.as_bytes())
                .for_each(|m| {
                    for capture in m.captures {
                        if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                            if capture_text == symbol_name {
                                candidates.push(capture.node);
                            }
                        }
                    }
                });
        }
    }
    
    // 2. Check local variable declarations in accessible scopes
    let mut current_node = Some(*usage_node);
    while let Some(node) = current_node {
        // Check if this node is a block or method body
        if matches!(node.kind(), "block" | "method_declaration" | "constructor_declaration") {
            find_local_variables_in_block(&node, source, symbol_name, usage_node, &mut candidates);
        }
        
        current_node = node.parent();
    }
    
    candidates
}

/// Find local variable declarations within a specific block that are accessible from the usage point
fn find_local_variables_in_block<'a>(
    block_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
    usage_node: &Node<'a>,
    candidates: &mut Vec<Node<'a>>,
) {
    let var_query = r#"(local_variable_declaration 
                        declarator: (variable_declarator 
                            name: (identifier) @name))"#;
    
    if let Ok(query) = get_or_create_query(var_query) {
        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, *block_node, source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                        if capture_text == symbol_name {
                            // Check if declaration comes before usage (same scope rule)
                            if capture.node.start_byte() < usage_node.start_byte() {
                                candidates.push(capture.node);
                            }
                        }
                    }
                }
            });
    }
}

/// Find the closest declaration based on scope distance
fn find_closest_declaration<'a>(
    usage_node: &Node<'a>,
    candidates: &[Node<'a>],
) -> Option<Node<'a>> {
    if candidates.is_empty() {
        return None;
    }
    
    // For now, return the last declaration (closest in scope)
    // TODO: Implement proper scope distance calculation
    candidates.last().copied()
}

/// Try to find symbol as a field declaration
fn find_as_field<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
) -> Option<Node<'a>> {
    let field_query = r#"(field_declaration 
                          declarator: (variable_declarator 
                              name: (identifier) @name))"#;
    
    if let Ok(query) = get_or_create_query(field_query) {
        let mut cursor = QueryCursor::new();
        let mut result = None;
        
        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                        if capture_text == symbol_name {
                            result = Some(capture.node);
                        }
                    }
                }
            });
        
        result
    } else {
        None
    }
}

/// Find the containing method for a given node
fn find_containing_method<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "method_declaration" | "constructor_declaration") {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Find the best method match for method calls
/// This handles method overloading by considering parameter types and count
fn find_best_method_match<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    method_name: &str,
) -> Option<Node<'a>> {
    // Find all method declarations with the same name
    let method_query = r#"(method_declaration name: (identifier) @name)"#;
    
    if let Ok(query) = get_or_create_query(method_query) {
        let mut cursor = QueryCursor::new();
        let mut candidates = Vec::new();
        
        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                        if capture_text == method_name {
                            // Find the parent method_declaration node
                            let mut parent = capture.node.parent();
                            while let Some(p) = parent {
                                if p.kind() == "method_declaration" {
                                    candidates.push(p);
                                    break;
                                }
                                parent = p.parent();
                            }
                        }
                    }
                }
            });
        
        // For now, return the first match
        // TODO: Implement proper method overloading resolution based on parameter types
        candidates.into_iter().next()
    } else {
        None
    }
}