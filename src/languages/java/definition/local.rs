use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::node_to_lsp_location},
    languages::LanguageSupport,
};

use super::definition_chain::{
    calculate_signature_match_score, extract_call_signature_from_context, extract_method_signature,
    find_method_with_signature, CallSignature,
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

    match symbol_type {
        SymbolType::MethodCall | SymbolType::MethodDeclaration => {
            // Use specialized method resolution for method calls
            find_best_method_match(tree, source, usage_node, symbol_name)
        }
        SymbolType::VariableUsage | SymbolType::VariableDeclaration => {
            // Search for variable declarations in accessible scopes
            let variable_candidates =
                find_variable_declarations_in_scope(tree, source, usage_node, &symbol_name);

            if !variable_candidates.is_empty() {
                // Find the best match using scope distance
                if let Some(best_match) = find_closest_declaration(usage_node, &variable_candidates)
                {
                    return Some(best_match);
                }
            }

            // Not found as variable, try as field
            find_as_field(tree, source, symbol_name)
        }

        SymbolType::FieldUsage | SymbolType::FieldDeclaration => {
            // Try to find as a field first, but don't fail if field query fails
            if let Some(query_text) = get_declaration_query_for_symbol_type(&symbol_type) {
                if let Some(candidates) =
                    find_definition_candidates(tree, source, &symbol_name, query_text)
                {
                    if let Some(field_match) = candidates.into_iter().next() {
                        return Some(field_match);
                    }
                }
            }

            // If not found as field, try as local variable (might be misclassified)
            let variable_candidates =
                find_variable_declarations_in_scope(tree, source, usage_node, &symbol_name);

            if !variable_candidates.is_empty() {
                // Find the best match using scope distance
                if let Some(best_match) = find_closest_declaration(usage_node, &variable_candidates)
                {
                    return Some(best_match);
                }
            }

            // If still not found, return None
            None
        }

        SymbolType::EnumDeclaration | SymbolType::EnumUsage => {
            // For enum types and enum constants
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;
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
#[tracing::instrument(skip_all)]
fn find_variable_declarations_in_scope<'a>(
    _tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Vec<Node<'a>> {
    let mut candidates = Vec::new();

    // 1. Check method parameters first (highest priority)
    if let Some(method) = find_containing_method(usage_node) {
        let param_query = r#"(parameter (identifier) @name)"#;
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
        if matches!(
            node.kind(),
            "block" | "function_declaration" | "constructor_declaration"
        ) {
            find_local_variables_in_block(&node, source, symbol_name, usage_node, &mut candidates);
        }

        current_node = node.parent();
    }

    candidates
}

/// Find local variable declarations within a specific block that are accessible from the usage point
#[tracing::instrument(skip_all)]
fn find_local_variables_in_block<'a>(
    block_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
    usage_node: &Node<'a>,
    candidates: &mut Vec<Node<'a>>,
) {
    let var_query = r#"(variable_declaration 
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
#[tracing::instrument(skip_all)]
fn find_closest_declaration<'a>(
    usage_node: &Node<'a>,
    candidates: &[Node<'a>],
) -> Option<Node<'a>> {
    if candidates.is_empty() {
        return None;
    }

    // Calculate scope distance for each candidate and find the closest
    let mut best_candidate = None;
    let mut best_distance = usize::MAX;

    for candidate in candidates {
        let distance = calculate_scope_distance(usage_node, candidate);
        if distance < best_distance {
            best_distance = distance;
            best_candidate = Some(*candidate);
        }
    }

    best_candidate
}

/// Calculate the scope distance between a usage node and a declaration node
/// Returns the number of scope levels between them (lower is closer)
#[tracing::instrument(skip_all)]
fn calculate_scope_distance(usage_node: &Node, declaration_node: &Node) -> usize {
    // Find common ancestor
    let mut usage_ancestors = Vec::new();
    let mut current = usage_node.parent();
    while let Some(parent) = current {
        usage_ancestors.push(parent);
        current = parent.parent();
    }

    let mut declaration_ancestors = Vec::new();
    current = declaration_node.parent();
    while let Some(parent) = current {
        declaration_ancestors.push(parent);
        current = parent.parent();
    }

    // Find the depth to common ancestor
    let mut common_ancestor_depth = 0;
    for (i, usage_ancestor) in usage_ancestors.iter().enumerate() {
        if declaration_ancestors.iter().any(|da| da.id() == usage_ancestor.id()) {
            common_ancestor_depth = i;
            break;
        }
    }

    // The scope distance is the depth from usage to common ancestor
    // Closer declarations (same block) have distance 0, outer scopes have higher distance
    common_ancestor_depth
}

/// Try to find symbol as a field declaration
#[tracing::instrument(skip_all)]
fn find_as_field<'a>(tree: &'a Tree, source: &str, symbol_name: &str) -> Option<Node<'a>> {
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
#[tracing::instrument(skip_all)]
fn find_containing_method<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "function_declaration" | "constructor_declaration"
        ) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Find the best method match for method calls
/// This handles method overloading by considering parameter types and count
/// Prioritizes methods in the same class before searching globally
#[tracing::instrument(skip_all)]
fn find_best_method_match<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    method_name: &str,
) -> Option<Node<'a>> {
    // Extract call signature from the method invocation context
    if let Some(call_signature) = extract_call_signature_from_context(usage_node, source) {
        // First, try to find methods with signature matching in the same class
        if let Some(same_class_method) = find_method_in_same_class_with_signature(
            usage_node,
            source,
            method_name,
            &call_signature,
        ) {
            return Some(same_class_method);
        }

        // If not found in the same class, search globally with signature matching
        if let Some(global_method) =
            find_method_with_signature(tree, source, method_name, &call_signature)
        {
            return Some(global_method);
        }
    }

    // Fallback: try without signature matching (same class first)
    if let Some(same_class_method) = find_method_in_same_class(usage_node, source, method_name) {
        return Some(same_class_method);
    }

    // Final fallback: search globally without signature
    find_method_globally(tree, source, method_name)
}

/// Find a method declaration within the same class using signature matching
#[tracing::instrument(skip_all)]
fn find_method_in_same_class_with_signature<'a>(
    usage_node: &Node<'a>,
    source: &str,
    method_name: &str,
    call_signature: &CallSignature,
) -> Option<Node<'a>> {
    // Find the containing class
    let containing_class = find_containing_class(usage_node)?;

    // Search for method declarations within this class with signature matching
    let method_query = r#"(function_declaration name: (identifier) @name)"#;

    if let Ok(query) = get_or_create_query(method_query) {
        let mut cursor = QueryCursor::new();
        let mut best_match = None;
        let mut best_score = 0;
        let mut fallback_match = None;

        cursor
            .matches(&query, containing_class, source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {

                        if capture_text == method_name {
                            // CRITICAL: Verify this is actually a method declaration name, not a method call
                            if let Some(parent) = capture.node.parent() {
                                if parent.kind() == "function_declaration" {
                                    // This is definitely a method declaration
                                    
                                    // Keep first match as fallback
                                    if fallback_match.is_none() {
                                        fallback_match = Some(capture.node);
                                    }

                                    // Try signature matching
                                    if let Some(method_sig) = extract_method_signature(&parent, source) {
                                        let score = calculate_signature_match_score(
                                            call_signature,
                                            &method_sig,
                                        );
                                        if score > best_score {
                                            best_score = score;
                                            best_match = Some(capture.node);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });

        // Return best signature match, or fallback if no good signature match
        if best_score > 0 {
            best_match
        } else {
            fallback_match
        }
    } else {
        None
    }
}

/// Find a method declaration within the same class as the usage node (name-only matching)
#[tracing::instrument(skip_all)]
fn find_method_in_same_class<'a>(
    usage_node: &Node<'a>,
    source: &str,
    method_name: &str,
) -> Option<Node<'a>> {
    // Find the containing class
    let containing_class = find_containing_class(usage_node)?;

    // Search for method declarations within this class
    let method_query = r#"(function_declaration name: (identifier) @name)"#;

    if let Ok(query) = get_or_create_query(method_query) {
        let mut cursor = QueryCursor::new();
        let mut candidates = Vec::new();

        cursor
            .matches(&query, containing_class, source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                        if capture_text == method_name {
                            // CRITICAL: Verify this is actually a method declaration name, not a method call
                            if let Some(parent) = capture.node.parent() {
                                if parent.kind() == "function_declaration" {
                                    // This is definitely a method declaration
                                    candidates.push(capture.node);
                                }
                            }
                        }
                    }
                }
            });

        // Return the first match in the same class
        candidates.into_iter().next()
    } else {
        None
    }
}

/// Find a method declaration globally across the entire tree
#[tracing::instrument(skip_all)]
fn find_method_globally<'a>(tree: &'a Tree, source: &str, method_name: &str) -> Option<Node<'a>> {
    let method_query = r#"(function_declaration name: (identifier) @name)"#;

    if let Ok(query) = get_or_create_query(method_query) {
        let mut cursor = QueryCursor::new();
        let mut candidates = Vec::new();

        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|m| {
                for capture in m.captures {
                    if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                        if capture_text == method_name {
                            // CRITICAL: Verify this is actually a method declaration name, not a method call
                            if let Some(parent) = capture.node.parent() {
                                if parent.kind() == "function_declaration" {
                                    // This is definitely a method declaration
                                    candidates.push(capture.node);
                                }
                            }
                        }
                    }
                }
            });

        // Return the first match globally
        candidates.into_iter().next()
    } else {
        None
    }
}

/// Find the containing class for a given node
#[tracing::instrument(skip_all)]
fn find_containing_class<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "class_declaration" | "enum_declaration" | "interface_declaration"
        ) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

