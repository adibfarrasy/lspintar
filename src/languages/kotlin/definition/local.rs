use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree};

use crate::{
    core::{utils::node_to_lsp_location, symbols::SymbolType},
    languages::LanguageSupport,
};

use super::utils::{find_definition_candidates, get_declaration_query_for_symbol_type};
use super::definition_chain::{extract_call_signature_from_context, find_method_with_signature};

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

        SymbolType::FieldUsage => {
            // Get candidates for field usage
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;

            // For field usage, find the field declaration
            candidates.into_iter().next()
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
fn find_variable_declarations_in_scope<'a>(
    _tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Vec<Node<'a>> {
    let mut candidates = Vec::new();

    // 1. Check function parameters first (highest priority)
    if let Some(function) = find_containing_function(usage_node) {
        if let Some(params) = find_function_parameters(&function, source, symbol_name) {
            candidates.extend(params);
        }
    }

    // 2. Check lambda parameters
    if let Some(lambda_params) = find_lambda_parameters(usage_node, source, symbol_name) {
        candidates.extend(lambda_params);
    }
    
    // 3. Check constructor parameters (for class members)
    if let Some(constructor_params) = find_constructor_parameters(usage_node, source, symbol_name) {
        candidates.extend(constructor_params);
    }

    // 4. Check for variable declarations in accessible scopes
    let mut current = usage_node.parent();
    while let Some(node) = current {
        // Check variable declarations in this scope that come before usage
        find_variables_in_block(&node, source, symbol_name, usage_node.start_byte(), &mut candidates);

        // Move to parent scope
        current = node.parent();
    }

    candidates
}

/// Find the closest declaration to the usage point
fn find_closest_declaration<'a>(
    usage_node: &Node,
    candidates: &[Node<'a>],
) -> Option<Node<'a>> {
    if candidates.is_empty() {
        return None;
    }

    let usage_start = usage_node.start_byte();
    
    // Find the candidate that is closest to (but before) the usage
    candidates.iter()
        .filter(|node| node.start_byte() < usage_start)
        .max_by_key(|node| node.start_byte())
        .copied()
}

/// Find variables as field declarations
fn find_as_field<'a>(tree: &'a Tree, source: &str, symbol_name: &str) -> Option<Node<'a>> {
    let query_text = r#"(property_declaration (variable_declaration (simple_identifier) @name))"#;
    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;
    candidates.into_iter().next()
}

/// Find the best method match using signature matching
fn find_best_method_match<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Option<Node<'a>> {
    // Extract call signature from the usage context
    let call_signature = extract_call_signature_from_context(usage_node, source);
    
    // Find method candidates
    let query_text = r#"(function_declaration (simple_identifier) @name)"#;
    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;
    
    if let Some(call_sig) = call_signature {
        // Try to find method with matching signature
        let result = find_method_with_signature(tree, source, symbol_name, &call_sig);
        if result.is_some() {
            result
        } else {
            // Signature matching failed, try first candidate
            candidates.into_iter().next()
        }
    } else {
        // No signature available, return first match
        candidates.into_iter().next()
    }
}

/// Find containing function for a node
fn find_containing_function<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_declaration" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Find function parameters that match the symbol name
fn find_function_parameters<'a>(
    function_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Option<Vec<Node<'a>>> {
    let mut parameters = Vec::new();
    
    for child in function_node.children(&mut function_node.walk()) {
        if child.kind() == "function_value_parameters" {
            for param_child in child.children(&mut child.walk()) {
                if param_child.kind() == "parameter" {
                    for param_part in param_child.children(&mut param_child.walk()) {
                        if param_part.kind() == "simple_identifier" {
                            if let Ok(param_name) = param_part.utf8_text(source.as_bytes()) {
                                if param_name == symbol_name {
                                    parameters.push(param_child);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    if parameters.is_empty() { None } else { Some(parameters) }
}

/// Find lambda parameters that match the symbol name
fn find_lambda_parameters<'a>(
    usage_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Option<Vec<Node<'a>>> {
    let mut current = usage_node.parent();
    let mut parameters = Vec::new();
    
    while let Some(node) = current {
        if node.kind() == "lambda_literal" {
            // Look for lambda parameters
            for child in node.children(&mut node.walk()) {
                if child.kind() == "lambda_parameters" {
                    for param_child in child.children(&mut child.walk()) {
                        if param_child.kind() == "lambda_parameter" {
                            for param_part in param_child.children(&mut param_child.walk()) {
                                if param_part.kind() == "simple_identifier" {
                                    if let Ok(param_name) = param_part.utf8_text(source.as_bytes()) {
                                        if param_name == symbol_name {
                                            parameters.push(param_child);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        current = node.parent();
    }
    
    if parameters.is_empty() { None } else { Some(parameters) }
}

/// Find variable declarations in a block that come before the usage
fn find_variables_in_block<'a>(
    block_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
    usage_byte_offset: usize,
    candidates: &mut Vec<Node<'a>>,
) {
    for child in block_node.children(&mut block_node.walk()) {
        if child.start_byte() >= usage_byte_offset {
            break; // Don't look at declarations after usage
        }
        
        match child.kind() {
            "property_declaration" => {
                if let Some(var_name) = get_declared_name(&child, source) {
                    if var_name == symbol_name {
                        candidates.push(child);
                    }
                }
            }
            "variable_declaration" => {
                if let Some(var_name) = get_declared_name(&child, source) {
                    if var_name == symbol_name {
                        candidates.push(child);
                    }
                }
            }
            "for_statement" => {
                // Check for loop variable declarations
                for for_child in child.children(&mut child.walk()) {
                    if for_child.kind() == "variable_declaration" || 
                       for_child.kind() == "multi_variable_declaration" {
                        if let Some(var_name) = get_declared_name(&for_child, source) {
                            if var_name == symbol_name {
                                candidates.push(for_child);
                            }
                        }
                    }
                }
            }
            "catch_block" => {
                // Check catch parameter
                for catch_child in child.children(&mut child.walk()) {
                    if catch_child.kind() == "simple_identifier" {
                        if let Ok(param_name) = catch_child.utf8_text(source.as_bytes()) {
                            if param_name == symbol_name {
                                candidates.push(child); // Return the catch_block itself
                            }
                        }
                    }
                }
            }
            _ => {
                // Recursively search in nested blocks
                find_variables_in_block(&child, source, symbol_name, usage_byte_offset, candidates);
            }
        }
    }
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

/// Find constructor parameters that match the symbol name
fn find_constructor_parameters<'a>(
    usage_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Option<Vec<Node<'a>>> {
    // Find the containing class
    let mut current = usage_node.parent();
    while let Some(node) = current {
        if node.kind() == "class_declaration" {
            // Look for primary_constructor
            for child in node.children(&mut node.walk()) {
                if child.kind() == "primary_constructor" {
                    return Some(find_matching_constructor_params(&child, source, symbol_name));
                }
            }
            break; // Don't look in parent classes
        }
        current = node.parent();
    }
    
    None
}

/// Find constructor parameters that match the given name
fn find_matching_constructor_params<'a>(
    constructor_node: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Vec<Node<'a>> {
    let mut params = Vec::new();
    
    // Look for class_parameter children
    for child in constructor_node.children(&mut constructor_node.walk()) {
        if child.kind() == "class_parameter" {
            // Extract the parameter name
            for param_child in child.children(&mut child.walk()) {
                if param_child.kind() == "simple_identifier" {
                    if let Ok(param_name) = param_child.utf8_text(source.as_bytes()) {
                        if param_name == symbol_name {
                            params.push(param_child);
                        }
                    }
                    break; // Only check the first identifier (the parameter name)
                }
            }
        }
    }
    
    params
}