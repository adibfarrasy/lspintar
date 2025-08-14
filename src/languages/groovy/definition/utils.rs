use std::{
    fs::{self, read_to_string},
    path::PathBuf,
};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
            uri_to_tree,
        },
    },
    languages::LanguageSupport,
};

use super::method_resolution::{extract_call_signature_from_context, find_method_with_signature};

#[tracing::instrument(skip_all)]
pub fn get_declaration_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration name: (identifier) @name)
            (interface_declaration name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
            (annotation_type_declaration name: (identifier) @name)
        "#,
        ),
        SymbolType::SuperClass => Some(r#"(class_declaration name: (identifier) @name)"#),
        SymbolType::SuperInterface => Some(r#"(interface_declaration name: (identifier) @name)"#),
        SymbolType::MethodCall => Some(r#"(method_declaration name: (identifier) @name)"#),
        SymbolType::FieldUsage => Some(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        ),
        SymbolType::VariableUsage => Some(
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
        "#,
        ),
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
pub fn find_definition_candidates<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    query_text: &str,
) -> Option<Vec<Node<'a>>> {
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let node_text = capture.node.utf8_text(source.as_bytes()).unwrap();

                if node_text == symbol_name {
                    candidates.push(capture.node.parent().unwrap());
                }
            }
        });

    Some(candidates)
}

#[tracing::instrument(skip_all)]
pub fn search_definition<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    symbol_type: SymbolType,
) -> Option<Node<'a>> {
    let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;

    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;

    candidates.into_iter().next()
}

#[tracing::instrument(skip_all)]
pub fn search_definition_in_project(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &Node,
    other_file_uri: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;
    let symbol_type = language_support
        .determine_symbol_type_from_context(&current_tree, usage_node, current_source)
        .ok()?;

    let other_tree = uri_to_tree(other_file_uri)?;
    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    let definition_node = if symbol_type == SymbolType::MethodCall {
        // For method calls, try signature-based matching first
        if let Some(call_signature) = extract_call_signature_from_context(usage_node, current_source) {
            find_method_with_signature(&other_tree, &other_source, symbol_name, &call_signature)
        } else {
            // Fallback to regular method search
            search_definition(&other_tree, &other_source, symbol_name, symbol_type)
        }
    } else {
        search_definition(&other_tree, &other_source, symbol_name, symbol_type)
    }?;

    return node_to_lsp_location(&definition_node, &other_file_uri);
}

/// Enhanced method resolution for static method calls like ObjectTransferUtil.transferObject()
pub fn search_static_method_definition_in_project(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &Node,
    other_file_uri: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;
    
    // Get the method invocation parent to extract call signature
    let method_invocation = find_parent_method_invocation_node(usage_node)?;
    let call_signature = extract_call_signature_from_context(usage_node, current_source)?;

    let other_tree = uri_to_tree(other_file_uri)?;
    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    // Use signature-based method matching
    let definition_node = find_method_with_signature(&other_tree, &other_source, symbol_name, &call_signature)?;

    return node_to_lsp_location(&definition_node, &other_file_uri);
}

fn find_parent_method_invocation_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_invocation" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Detect if a method call is static and extract the class name
pub fn extract_static_method_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let method_invocation = find_parent_method_invocation_node(usage_node)?;
    
    // Check if this method invocation has an object field (static method pattern)
    let object_node = method_invocation.child_by_field_name("object")?;
    let method_name_node = method_invocation.child_by_field_name("name")?;
    
    let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let method_name = method_name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    
    // Verify that the usage_node is the method name part
    let usage_text = usage_node.utf8_text(source.as_bytes()).ok()?;
    if usage_text == method_name {
        Some((class_name, method_name))
    } else {
        None
    }
}

/// Detect if a method call is on an instance and extract the variable name
pub fn extract_instance_method_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let method_invocation = find_parent_method_invocation_node(usage_node)?;
    
    // Check if this method invocation has an object field (instance method pattern)
    let object_node = method_invocation.child_by_field_name("object")?;
    let method_name_node = method_invocation.child_by_field_name("name")?;
    
    let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let method_name = method_name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    
    // Verify that the usage_node is the method name part
    let usage_text = usage_node.utf8_text(source.as_bytes()).ok()?;
    if usage_text == method_name {
        // Check if the object looks like a variable (lowercase first letter) vs class (uppercase first letter)
        if variable_name.chars().next()?.is_lowercase() {
            Some((variable_name, method_name))
        } else {
            None // This looks like a static method call
        }
    } else {
        None
    }
}

/// Resolve a variable to find its type/class name
pub fn resolve_variable_type(variable_name: &str, tree: &Tree, source: &str, current_position: &Node) -> Option<String> {
    
    // Look for field declarations (class properties)
    if let Some(field_type) = find_field_declaration_type(variable_name, tree, source) {
        return Some(field_type);
    }
    
    // Look for variable declarations in scope
    if let Some(var_type) = find_variable_declaration_type(variable_name, tree, source, current_position) {
        return Some(var_type);
    }
    
    // Try to infer from method parameters
    if let Some(param_type) = find_parameter_type(variable_name, tree, source, current_position) {
        return Some(param_type);
    }
    
    // Try to infer from assignment expressions
    if let Some(assignment_type) = infer_from_assignment(variable_name, tree, source, current_position) {
        return Some(assignment_type);
    }
    
    None
}

fn find_field_declaration_type(variable_name: &str, tree: &Tree, source: &str) -> Option<String> {
    
    let query_text = r#"
        (field_declaration 
          type: (type_identifier) @field_type
          declarator: (variable_declarator 
            name: (identifier) @field_name))
        
        (field_declaration 
          declarator: (variable_declarator 
            name: (identifier) @field_name_untyped))
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut processed_nodes = std::collections::HashSet::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        let mut field_name_node = None;
        let mut field_type = None;
        let mut found_target = false;

        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            // Skip if we've already processed this node
            let node_id = capture.node.id();
            if processed_nodes.contains(&node_id) {
                continue;
            }

            match capture_name {
                "field_name" | "field_name_untyped" => {
                    processed_nodes.insert(node_id);
                    if node_text == variable_name {
                        field_name_node = Some(capture.node);
                        found_target = true;
                    }
                }
                "field_type" => {
                    field_type = Some(node_text.to_string());
                }
                _ => {}
            }
        }

        // Early termination: if we found our target field, process it immediately
        if found_target {
            if let Some(type_name) = field_type {
                return Some(type_name);
            } else {
                // For Spring-injected fields, try to infer type from field name
                let inferred_type = infer_type_from_field_name(variable_name);
                if let Some(type_name) = inferred_type {
                    return Some(type_name);
                }
            }
        }
    }

    None
}

fn find_variable_declaration_type(variable_name: &str, tree: &Tree, source: &str, current_position: &Node) -> Option<String> {
    
    let query_text = r#"
        (variable_declaration 
          type: (type_identifier) @var_type
          declarator: (variable_declarator 
            name: (identifier) @var_name))
        
        (variable_declaration 
          declarator: (variable_declarator 
            name: (identifier) @var_name_untyped))
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        let mut var_name = None;
        let mut var_type = None;
        let mut found_target = false;

        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "var_name" | "var_name_untyped" => {
                    if node_text == variable_name {
                        var_name = Some(capture.node);
                        found_target = true;
                    }
                }
                "var_type" => {
                    var_type = Some(node_text.to_string());
                }
                _ => {}
            }
        }

        // Early termination: if we found our target variable, process it immediately
        if found_target {
            if let Some(name_node) = var_name {
                if is_variable_in_scope(&name_node, current_position) {
                    if let Some(type_name) = var_type {
                        return Some(type_name);
                    } else {
                        // Try to infer from initializer
                        if let Some(inferred_type) = infer_type_from_initializer(&name_node, source) {
                            return Some(inferred_type);
                        }
                    }
                }
            }
        }
    }

    None
}

fn find_parameter_type(variable_name: &str, tree: &Tree, source: &str, current_position: &Node) -> Option<String> {
    let query_text = r#"
        (formal_parameter 
          type: (type_identifier) @param_type
          name: (identifier) @param_name)
        
        (formal_parameter 
          name: (identifier) @param_name_untyped)
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut result = None;
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            let mut param_name = None;
            let mut param_type = None;

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "param_name" | "param_name_untyped" => {
                        if node_text == variable_name {
                            param_name = Some(capture.node);
                        }
                    }
                    "param_type" => {
                        param_type = Some(node_text.to_string());
                    }
                    _ => {}
                }
            }

            // Check if this parameter is in the same method as current position
            if let Some(name_node) = param_name {
                if is_in_same_method(&name_node, current_position) {
                    if let Some(type_name) = param_type {
                        result = Some(type_name);
                        return;
                    }
                }
            }
        });

    result
}

fn infer_from_assignment(variable_name: &str, tree: &Tree, source: &str, current_position: &Node) -> Option<String> {
    let query_text = r#"
        (assignment_expression 
          left: (identifier) @var_name
          right: (object_creation_expression 
            type: (type_identifier) @assigned_type))
            
        (assignment_expression 
          left: (identifier) @var_name
          right: (method_invocation) @method_call)
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut result = None;
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            let mut var_name = None;
            let mut assigned_type = None;

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "var_name" => {
                        if node_text == variable_name {
                            var_name = Some(capture.node);
                        }
                    }
                    "assigned_type" => {
                        assigned_type = Some(node_text.to_string());
                    }
                    "method_call" => {
                        // Could try to infer return type, but that's complex
                        // For now, skip method calls
                    }
                    _ => {}
                }
            }

            // Check if this assignment is before current position and in scope
            if let Some(name_node) = var_name {
                if is_assignment_before_position(&name_node, current_position) {
                    if let Some(type_name) = assigned_type {
                        result = Some(type_name);
                        return;
                    }
                }
            }
        });

    result
}

fn infer_type_from_initializer(var_node: &Node, source: &str) -> Option<String> {
    // Look for variable declarator with initializer
    let var_declarator = var_node.parent()?;
    if var_declarator.kind() != "variable_declarator" {
        return None;
    }

    let initializer = var_declarator.child_by_field_name("value")?;
    
    match initializer.kind() {
        "object_creation_expression" => {
            if let Some(type_node) = initializer.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(type_text.to_string())
            } else {
                None
            }
        }
        "string_literal" => Some("String".to_string()),
        "decimal_integer_literal" => Some("Integer".to_string()),
        "decimal_floating_point_literal" => Some("Double".to_string()),
        "true" | "false" => Some("Boolean".to_string()),
        _ => None
    }
}

fn is_variable_in_scope(var_node: &Node, current_position: &Node) -> bool {
    // Simple check: variable declaration should come before current position
    var_node.start_position() < current_position.start_position()
}

fn is_in_same_method(param_node: &Node, current_position: &Node) -> bool {
    let param_method = find_containing_method_node(param_node);
    let current_method = find_containing_method_node(current_position);
    
    match (param_method, current_method) {
        (Some(p_method), Some(c_method)) => p_method.id() == c_method.id(),
        _ => false
    }
}

fn is_assignment_before_position(assignment_node: &Node, current_position: &Node) -> bool {
    assignment_node.start_position() < current_position.start_position()
}

fn infer_type_from_field_name(field_name: &str) -> Option<String> {
    // Convert camelCase field name to PascalCase class name
    // Examples:
    // apiOrderTransfer -> ApiOrderTransfer
    // userService -> UserService
    // orderRepository -> OrderRepository
    
    if field_name.is_empty() {
        return None;
    }
    
    let mut result = String::new();
    let mut chars = field_name.chars();
    
    // Capitalize the first character
    if let Some(first_char) = chars.next() {
        result.push(first_char.to_uppercase().next().unwrap_or(first_char));
    }
    
    // Add the rest of the characters
    for ch in chars {
        result.push(ch);
    }
    
    Some(result)
}

fn find_containing_method_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_declaration" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

#[tracing::instrument(skip_all)]
pub fn prepare_symbol_lookup_key(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root
        .or_else(|| find_project_root(&current_file_path))
        .or_else(|| find_external_dependency_root(&current_file_path))?;

    resolve_through_imports(&symbol_name, source, &project_root)
        .or_else(|| resolve_same_package(&symbol_name, source, &project_root, dependency_cache))
}

/// Enhanced symbol lookup that supports wildcard imports using the class name index
pub fn prepare_symbol_lookup_key_with_wildcard_support(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root
        .or_else(|| find_project_root(&current_file_path))
        .or_else(|| find_external_dependency_root(&current_file_path))?;

    // First try regular resolution (specific imports and same package)
    let specific_import_result = resolve_through_imports(&symbol_name, source, &project_root);
    if let Some(result) = &specific_import_result {
        return Some(result.clone());
    }
    
    let same_package_result = resolve_same_package(&symbol_name, source, &project_root, dependency_cache);
    if let Some(result) = &same_package_result {
        return Some(result.clone());
    }

    // If not found, try wildcard import resolution
    resolve_through_wildcard_imports(&symbol_name, source, &project_root, dependency_cache)
}

fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (import_declaration) @import_decl
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut specific_import = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                    // Extract just the import path from "import com.example.package.*"
                    let import_text = full_import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    // Only handle specific imports here - wildcard imports are handled in resolve_through_wildcard_imports
                    if import_text.ends_with(&format!(".{}", symbol_name)) {
                        specific_import = Some((project_root.clone(), import_text.to_string()));
                        return;
                    }
                };
            }
        });

    // Return specific import if found - do NOT return wildcard candidates here
    specific_import
}

fn resolve_through_wildcard_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    // Get all wildcard imports from the source
    let wildcard_packages = get_wildcard_imports(source)?;
    
    // Get all possible FQNs for this class name in the project
    let possible_fqns = dependency_cache.find_symbols_by_class_name(project_root, symbol_name);
    
    // Check if any of the possible FQNs match any wildcard import
    for fqn in possible_fqns {
        for wildcard_package in &wildcard_packages {
            let expected_prefix = format!("{}.", wildcard_package);
            if fqn.starts_with(&expected_prefix) {
                return Some((project_root.clone(), fqn));
            }
        }
    }
    
    None
}

pub fn get_wildcard_imports_from_source(source: &str) -> Option<Vec<String>> {
    get_wildcard_imports(source)
}

fn get_wildcard_imports(source: &str) -> Option<Vec<String>> {
    let query_text = r#"
        (import_declaration) @import_decl
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut wildcard_packages = Vec::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                    // Extract just the import path from "import com.example.package.*"
                    let import_text = full_import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    if import_text.ends_with("*") {
                        let package_name = import_text.strip_suffix("*").unwrap_or(import_text);
                        let package_name = package_name.trim_end_matches('.');
                        wildcard_packages.push(package_name.to_string());
                    }
                }
            }
        });

    Some(wildcard_packages)
}

pub fn set_start_position(source: &str, usage_node: &Node, file_uri: &str) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    let other_source = fs::read_to_string(uri_to_path(file_uri)?).ok()?;

    let query_text = r#"
      (identifier) @name 
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&other_source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), other_source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                // Already found a match
                return;
            }

            for capture in query_match.captures {
                if let Ok(name) = capture.node.utf8_text(other_source.as_bytes()) {
                    if name == symbol_name {
                        result = node_to_lsp_location(&capture.node, file_uri)
                    }
                };
            }
        });

    result
}

fn resolve_same_package(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (package_declaration
          (scoped_identifier) @package_name)
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                // Already found a match
                // should only have 1 match
                return;
            }

            for capture in query_match.captures {
                if let Ok(package_name) = capture.node.utf8_text(source.as_bytes()) {
                    let fqn = format!("{}.{}", package_name, symbol_name);
                    
                    // Only return the same-package result if the symbol actually exists in the cache
                    let symbol_key = (project_root.clone(), fqn.clone());
                    if dependency_cache.symbol_index.get(&symbol_key).is_some() {
                        result = Some((project_root.clone(), fqn));
                        return;
                    }
                };
            }
        });

    result
}
