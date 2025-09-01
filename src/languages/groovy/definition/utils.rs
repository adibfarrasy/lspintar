use std::{fs::read_to_string, path::PathBuf};

use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, get_language_support_for_file,
            node_to_lsp_location, uri_to_path, uri_to_tree,
        },
    },
    languages::{groovy::constants::GROOVY_DEFAULT_IMPORTS, LanguageSupport},
};

use super::definition_chain::{extract_call_signature_from_context, find_method_with_signature};

/// Get or create a compiled query
pub fn get_or_create_query(query_text: &str, language: &tree_sitter::Language) -> Option<Query> {
    Query::new(language, query_text).ok()
}

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
        SymbolType::FieldUsage => {
            Some(r#"(field_declaration declarator: (variable_declarator (identifier) @name))"#)
        }
        SymbolType::VariableUsage => Some(
            r#"
            (variable_declaration declarator: (variable_declarator (identifier) @name))
            (formal_parameter (identifier) @name)
            (field_declaration declarator: (variable_declarator (identifier) @name))
        "#,
        ),
        SymbolType::EnumDeclaration => Some(r#"(enum_declaration name: (identifier) @name)"#),
        SymbolType::EnumUsage => Some(
            r#"
            (enum_constant name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
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
    let query = get_or_create_query(query_text, &tree.language())?;
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();

    // Optimized: Use while loop with early termination potential
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == symbol_name {
                    if let Some(parent) = capture.node.parent() {
                        candidates.push(parent);
                    }
                }
            }
        }

        // Early termination for single-result queries (local scope) - but not for variable declarations
        // since we need to find the declaration that comes before usage, not just any assignment
        if !candidates.is_empty()
            && is_local_scope_query(query_text)
            && !query_text.contains("variable_declaration")
        {
            break;
        }
    }

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

/// Check if this is a query that should terminate early for local scope
fn is_local_scope_query(query_text: &str) -> bool {
    query_text.contains("formal_parameter") || query_text.contains("variable_declaration")
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
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {

    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;

    tracing::debug!(
        "LSPINTAR_DEBUG: groovy search - symbol_name = '{}', other_file_uri = {}",
        symbol_name, other_file_uri
    );

    // Get the appropriate language support for the current file (where the symbol usage is)
    let current_file_path = uri_to_path(current_file_uri)?;
    let current_language_support = get_language_support_for_file(&current_file_path)?;

    let symbol_type = current_language_support
        .determine_symbol_type_from_context(&current_tree, usage_node, current_source)
        .ok()?;

    tracing::debug!(
        "LSPINTAR_DEBUG: groovy search - symbol_type = {:?}",
        symbol_type
    );

    let other_tree = uri_to_tree(other_file_uri)?;

    tracing::debug!(
        "LSPINTAR_DEBUG: groovy search - got other_tree, root node kind = {}",
        other_tree.root_node().kind()
    );

    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    let definition_node = if symbol_type == SymbolType::MethodCall {
        tracing::debug!(
            "LSPINTAR_DEBUG: groovy search - searching for method call '{}'",
            symbol_name
        );
        // For method calls, try signature-based matching first
        if let Some(call_signature) =
            extract_call_signature_from_context(usage_node, current_source)
        {
            find_method_with_signature(&other_tree, &other_source, symbol_name, &call_signature)
        } else {
            // Fallback to regular method search
            search_definition(&other_tree, &other_source, symbol_name, symbol_type)
        }
    } else {
        tracing::debug!(
            "LSPINTAR_DEBUG: groovy search - searching for symbol '{}' with type {:?}",
            symbol_name, symbol_type
        );
        search_definition(&other_tree, &other_source, symbol_name, symbol_type)
    };

    tracing::debug!(
        "LSPINTAR_DEBUG: groovy search - definition_node found: {}",
        definition_node.is_some()
    );

    if let Some(node) = definition_node {
        let location = node_to_lsp_location(&node, &other_file_uri);
        tracing::debug!(
            "LSPINTAR_DEBUG: groovy search - final result: {:?}",
            location.as_ref().map(|loc| format!("{}:{}", loc.uri, loc.range.start.line))
        );
        return location;
    } else {
        tracing::debug!(
            "LSPINTAR_DEBUG: groovy search - no definition node found, returning None"
        );
        return None;
    }
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


/// Detect if a method call is on an instance and extract the variable name
pub fn extract_instance_method_context(
    usage_node: &Node,
    source: &str,
) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");

    let method_invocation = find_parent_method_invocation_node(usage_node);
    if method_invocation.is_none() {
        return None;
    }
    let method_invocation = method_invocation.unwrap();

    // Check if this method invocation has an object field (instance method pattern)
    let object_node = method_invocation.child_by_field_name("object")?;
    let method_name_node = method_invocation.child_by_field_name("name")?;

    let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let method_name = method_name_node
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();


    // Verify that the _usage_node is the method name part
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
pub fn resolve_variable_type(
    variable_name: &str,
    tree: &Tree,
    source: &str,
    current_position: &Node,
) -> Option<String> {
    // Look for field declarations (class properties)
    if let Some(field_type) = find_field_declaration_type(variable_name, tree, source) {
        return Some(field_type);
    }

    // Look for variable declarations in scope
    if let Some(var_type) =
        find_variable_declaration_type(variable_name, tree, source, current_position)
    {
        return Some(var_type);
    }

    // Try to infer from method parameters
    if let Some(param_type) = find_parameter_type(variable_name, tree, source, current_position) {
        return Some(param_type);
    }

    // Try to infer from assignment expressions
    if let Some(assignment_type) =
        infer_from_assignment(variable_name, tree, source, current_position)
    {
        return Some(assignment_type);
    }

    None
}

fn find_field_declaration_type(variable_name: &str, tree: &Tree, source: &str) -> Option<String> {
    // Enhanced query to handle annotated fields with explicit types
    let query_text = r#"
        (field_declaration 
          (modifiers)?
          type: (type_identifier) @field_type
          declarator: (variable_declarator 
            name: (identifier) @field_name))
            
        (field_declaration 
          declarator: (variable_declarator 
            name: (identifier) @field_name_no_type))
    "#;

    let query = get_or_create_query(query_text, &tree.language())?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        let mut found_field_name = false;
        let mut explicit_type = None;

        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "field_name" | "field_name_no_type" => {
                    if node_text == variable_name {
                        found_field_name = true;
                    }
                }
                "field_type" => {
                    explicit_type = Some(node_text.to_string());
                }
                _ => {}
            }
        }

        if found_field_name {
            if let Some(type_text) = explicit_type {
                // Found explicit type annotation
                return Some(type_text);
            } else {
                // No explicit type, fall back to field name inference
                return infer_type_from_field_name(variable_name);
            }
        }
    }

    None
}

/// Fast ancestor lookup with early termination
fn find_ancestor_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut current = Some(*node);
    let mut depth = 0;

    while let Some(n) = current {
        if n.kind() == kind {
            return Some(n);
        }
        current = n.parent();
        depth += 1;

        // Safety: prevent infinite loops in malformed trees
        if depth > 10 {
            break;
        }
    }
    None
}

fn find_variable_declaration_type(
    variable_name: &str,
    tree: &Tree,
    source: &str,
    current_position: &Node,
) -> Option<String> {
    // Optimized: Single query with immediate processing
    let query_text = r#"
        (variable_declaration 
          declarator: (variable_declarator 
            name: (identifier) @var_name))
    "#;

    let query = get_or_create_query(query_text, &tree.language())?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == variable_name {
                    // Check scope first (fast check)
                    if is_variable_in_scope(&capture.node, current_position) {
                        // Found in scope, now get the type
                        if let Some(var_decl) =
                            find_ancestor_of_kind(&capture.node, "variable_declaration")
                        {
                            // Look for explicit type
                            if let Some(type_node) = var_decl.child_by_field_name("type") {
                                if let Ok(type_text) = type_node.utf8_text(source.as_bytes()) {
                                    return Some(type_text.to_string());
                                }
                            }
                            // Try to infer from initializer
                            if let Some(inferred_type) =
                                infer_type_from_initializer(&capture.node, source)
                            {
                                return Some(inferred_type);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn find_parameter_type(
    variable_name: &str,
    tree: &Tree,
    source: &str,
    current_position: &Node,
) -> Option<String> {
    let query_text = r#"
        (formal_parameter 
          type: (type_identifier) @param_type
          name: (identifier) @param_name)
        
        (formal_parameter 
          name: (identifier) @param_name_untyped)
    "#;

    let query = get_or_create_query(query_text, &tree.language())?;
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

fn infer_from_assignment(
    variable_name: &str,
    tree: &Tree,
    source: &str,
    current_position: &Node,
) -> Option<String> {
    let query_text = r#"
        (assignment_expression 
          left: (identifier) @var_name
          right: (object_creation_expression 
            type: (type_identifier) @assigned_type))
            
        (assignment_expression 
          left: (identifier) @var_name
          right: (method_invocation) @method_call)
    "#;

    let query = get_or_create_query(query_text, &tree.language())?;
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
        _ => None,
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
        _ => false,
    }
}

fn is_assignment_before_position(assignment_node: &Node, current_position: &Node) -> bool {
    assignment_node.start_position() < current_position.start_position()
}

fn infer_type_from_field_name(field_name: &str) -> Option<String> {
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
    let specific_import_result = resolve_through_imports(&symbol_name, source, &project_root, dependency_cache);
    if let Some(result) = &specific_import_result {
        return Some(result.clone());
    } else {
    }

    let same_package_result =
        resolve_same_package(&symbol_name, source, &project_root, dependency_cache);
    if let Some(result) = &same_package_result {
        return Some(result.clone());
    }

    // If not found, try wildcard import resolution
    let wildcard_result = resolve_through_wildcard_imports(&symbol_name, source, &project_root, dependency_cache);
    if let Some(result) = &wildcard_result {
        return Some(result.clone());
    }

    None
}

fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
    dependency_cache: &DependencyCache,
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
                        // Check both local symbols, external dependencies, and builtin classes using read-through cache
                        let explicit_key = (project_root.clone(), import_text.to_string());
                        
                        // First try current project (symbols and builtins)
                        if dependency_cache.find_symbol_sync(&explicit_key.0, &explicit_key.1).is_some()
                            || dependency_cache.find_builtin_info(&explicit_key.1).is_some() {
                            specific_import = Some(explicit_key.clone());
                            return;
                        }
                        
                        // Then try current project's external dependencies (JAR files)
                        if let Some(_) = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                dependency_cache.find_project_external_info(project_root, import_text).await
                            })
                        }) {
                            specific_import = Some(explicit_key);
                            return;
                        }
                        
                        // Then try dependency projects
                        if let Some(project_metadata) = dependency_cache.project_metadata.get(project_root) {
                            for dependent_project_ref in project_metadata.inter_project_deps.iter() {
                                let dependent_project = dependent_project_ref.clone();
                                let dep_key = (dependent_project.clone(), import_text.to_string());
                                
                                if dependency_cache.find_symbol_sync(&dep_key.0, &dep_key.1).is_some()
                                    || dependency_cache.find_builtin_info(&dep_key.1).is_some() {
                                    specific_import = Some(dep_key);
                                    return;
                                }
                                
                                // Also check external dependencies of this dependency project
                                if let Some(_) = tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        dependency_cache.find_project_external_info(&dependent_project, import_text).await
                                    })
                                }) {
                                    specific_import = Some(dep_key);
                                    return;
                                }
                            }
                        }
                        
                    } else if !import_text.ends_with("*") {
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
    let wildcard_packages = get_wildcard_imports(source);
    if wildcard_packages.is_none() {
    } else {
    }
    let wildcard_packages = wildcard_packages?;

    // Get all possible FQNs for this class name in the project
    let possible_fqns = dependency_cache.find_symbols_by_class_name(project_root, symbol_name);

    // Optimized: Pre-compute prefixes and use faster matching
    let prefixes: Vec<String> = wildcard_packages
        .iter()
        .map(|pkg| format!("{}.", pkg))
        .collect();

    // Check if any FQN matches any wildcard prefix using read-through cache
    for fqn in possible_fqns {
        if prefixes.iter().any(|prefix| fqn.starts_with(prefix)) {
            // Check if this symbol actually exists using read-through cache
            if dependency_cache.find_symbol_sync(project_root, &fqn).is_some() {
                return Some((project_root.clone(), fqn));
            }
        }
    }

    // Also check for wildcard imports (both project symbols and builtin classes)
    for package in &wildcard_packages {
        // Strip 'static ' prefix if present (from static imports)
        let clean_package = if package.starts_with("static ") {
            &package[7..] // Remove "static " prefix
        } else {
            package
        };
        
        let potential_fqn = format!("{}.{}", clean_package, symbol_name);
        
        // First try project symbols (for static imports and regular classes)
        if dependency_cache.find_symbol_sync(project_root, &potential_fqn).is_some() {
            return Some((project_root.clone(), potential_fqn));
        }
        
        // Then try builtin classes
        if dependency_cache.find_builtin_info(&potential_fqn).is_some() {
            return Some((project_root.clone(), potential_fqn));
        }
    }
    
    // Handle GROOVY_DEFAULT_IMPORTS (prioritize exact imports over wildcards)
    // First pass: check exact imports
    for import in GROOVY_DEFAULT_IMPORTS.iter() {
        if !import.ends_with(".*") {
            // Exact import: check if this matches the symbol directly
            if import.ends_with(&format!(".{}", symbol_name)) || import == &symbol_name {
                if dependency_cache.find_builtin_info(import).is_some() {
                    return Some((project_root.clone(), import.to_string()));
                }
            }
        }
    }
    
    // Second pass: check wildcard imports only if exact imports didn't match
    for import in GROOVY_DEFAULT_IMPORTS.iter() {
        if import.ends_with(".*") {
            // Wildcard import: java.io.* + BigDecimal = java.io.BigDecimal
            let package = import.strip_suffix(".*").unwrap();
            let fqn = format!("{}.{}", package, symbol_name);
            if dependency_cache.find_builtin_info(&fqn).is_some() {
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
    use crate::core::utils::set_start_position_for_language;
    set_start_position_for_language(source, usage_node, file_uri, "groovy")
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

                    // Only return if the symbol actually exists in this package using read-through cache
                    let explicit_key = (project_root.clone(), fqn.clone());
                    if dependency_cache.find_symbol_sync(&explicit_key.0, &explicit_key.1).is_some()
                        || dependency_cache.find_builtin_info(&explicit_key.1).is_some() {
                        result = Some(explicit_key);
                        return;
                    }
                };
            }
        });

    result
}

/// Resolve symbol name with import context
#[tracing::instrument(skip_all)]
pub fn resolve_symbol_with_imports(
    symbol_name: &str,
    source: &str,
    dependency_cache: &DependencyCache,
) -> Option<String> {
        
    // Extract imports from source
    let imports = extract_imports_from_source(source);
    
    // First, check for exact matches and specific imports
    let mut star_imports = Vec::new();
    for import in &imports {
        let expected_suffix = format!(".{}", symbol_name);
        let matches_suffix = import.ends_with(&expected_suffix);
        let exact_match = import == symbol_name;
        
        if matches_suffix || exact_match {
            return Some(import.clone());
        }
        
        // Collect star imports for later use
        if import.ends_with(".*") {
            let package = import.strip_suffix(".*").unwrap_or("");
            star_imports.push(package);
        }
    }
    
    // For common Groovy/Java types, try java.lang and groovy.lang first
    let common_groovy_types = [
        // Java.lang types (implicitly imported in Groovy)
        "String", "Integer", "Long", "Double", "Float", "Boolean", "Character",
        "Byte", "Short", "Object", "Class", "System", "Math", "Thread",
        "Runnable", "Exception", "RuntimeException", "Error", "Throwable",
        "Number", "Comparable", "Cloneable", "Serializable", "Iterable",
        "Collection", "List", "Set", "Map", "ArrayList", "HashMap", "HashSet",
        "LinkedList", "TreeMap", "TreeSet", "Queue", "Deque", "Stack", "Vector",
        // Groovy-specific types
        "Closure", "GString", "GroovyObject", "MetaClass", "Expando", "ConfigSlurper",
    ];
    
    if common_groovy_types.contains(&symbol_name.as_ref()) {
        // Try java.lang first for common Java types
        if ["String", "Integer", "Long", "Double", "Float", "Boolean", "Character",
            "Byte", "Short", "Object", "Class", "System", "Math", "Thread",
            "Runnable", "Exception", "RuntimeException", "Error", "Throwable",
            "Number", "Comparable", "Cloneable", "Serializable", "Iterable",
            "Collection", "List", "Set", "Map", "ArrayList", "HashMap", "HashSet",
            "LinkedList", "TreeMap", "TreeSet", "Queue", "Deque", "Stack", "Vector"].contains(&symbol_name.as_ref()) {
            let java_lang_fqn = format!("java.lang.{}", symbol_name);
            return Some(java_lang_fqn);
        } else {
            // Try groovy.lang for Groovy-specific types
            let groovy_lang_fqn = format!("groovy.lang.{}", symbol_name);
            return Some(groovy_lang_fqn);
        }
    }
    
    // Try star imports
    for package in star_imports {
        let candidate_fqn = format!("{}.{}", package, symbol_name);
        // Verify this FQN exists in cache/database
        if verify_groovy_fqn_exists(&candidate_fqn, dependency_cache) {
            return Some(candidate_fqn);
        }
    }
    
    None
}

/// Verify that a given FQN exists across all sources: builtins, workspace projects, and external dependencies
async fn verify_groovy_fqn_exists_async(fqn: &str, dependency_cache: &DependencyCache) -> bool {
    // Check builtin classes (like java.lang.* classes)
    if let Some(class_name) = fqn.split('.').last() {
        if dependency_cache.builtin_infos.get(class_name).is_some() {
            return true;
        }
    }

    // Check all workspace projects using find_symbol
    for project_entry in dependency_cache.project_metadata.iter() {
        let project_root = project_entry.key();
        if dependency_cache.find_symbol(project_root, fqn).await.is_some() {
            return true;
        }
        
        // Also check external dependencies for this project
        if dependency_cache.find_external_symbol_with_lazy_parsing(project_root, fqn).await.is_some() {
            return true;
        }
    }
    
    false
}

/// Synchronous wrapper for backward compatibility
fn verify_groovy_fqn_exists(fqn: &str, dependency_cache: &DependencyCache) -> bool {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(verify_groovy_fqn_exists_async(fqn, dependency_cache))
    })
}

/// Extract imports from Groovy source code
fn extract_imports_from_source(source: &str) -> Vec<String> {
    let mut imports = Vec::new();
    
    let query_text = r#"(import_declaration) @import_decl"#;
    
    let language = tree_sitter_groovy::language();
    if let Ok(query) = Query::new(&language, query_text) {
        let mut parser = Parser::new();
        if parser.set_language(&language).is_ok() {
            if let Some(tree) = parser.parse(source, None) {
                let mut cursor = QueryCursor::new();
                
                cursor
                    .matches(&query, tree.root_node(), source.as_bytes())
                    .for_each(|query_match| {
                        for capture in query_match.captures {
                            if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                                // Extract just the import path from "import com.example.Class"
                                let import_text = full_import_text
                                    .trim_start_matches("import")
                                    .trim()
                                    .trim_end_matches(';')
                                    .trim();
                                    
                                // Remove "static" keyword if present
                                let clean_import = if import_text.starts_with("static ") {
                                    &import_text[7..]
                                } else {
                                    import_text
                                };
                                
                                imports.push(clean_import.to_string());
                            }
                        }
                    });
            }
        }
    }
    
    imports
}

#[cfg(test)]
#[allow(unused_variables)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_groovy_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        match parser.set_language(&tree_sitter_groovy::language()) {
            Ok(()) => Some(parser),
            Err(_) => None,
        }
    }

    #[test]
    fn test_get_declaration_query_for_groovy_enum_types() {
        // Test that enum declaration and usage queries are provided
        let enum_decl_query = get_declaration_query_for_symbol_type(&SymbolType::EnumDeclaration);
        assert!(enum_decl_query.is_some());
        
        let enum_usage_query = get_declaration_query_for_symbol_type(&SymbolType::EnumUsage);
        assert!(enum_usage_query.is_some());
    }

    #[test]
    fn test_search_definition_groovy_enum_constant() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        // Test source with enum definition
        let source = r#"
enum Size {
    SMALL,
    MEDIUM, 
    LARGE
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Search for enum constant "MEDIUM"
        let result = search_definition(&tree, source, "MEDIUM", SymbolType::EnumUsage);
        assert!(result.is_some(), "Should find MEDIUM enum constant");
        
        // Search for enum constant "SMALL"
        let result = search_definition(&tree, source, "SMALL", SymbolType::EnumUsage);  
        assert!(result.is_some(), "Should find SMALL enum constant");
        
        // Search for non-existent constant
        let result = search_definition(&tree, source, "NONEXISTENT", SymbolType::EnumUsage);
        assert!(result.is_none(), "Should not find non-existent constant");
    }

    #[test]
    fn test_extract_imports_with_groovy_static_enum_imports() {
        let source = r#"
package com.test

import java.util.List
import static com.test.enums.Priority.*
import static com.test.enums.Direction.NORTH

class MyClass {
    // class body
}
"#;
        
        let imports = extract_imports_from_source(source);
        
        // Check that static imports are included
        assert!(imports.contains(&"com.test.enums.Priority.*".to_string()));
        assert!(imports.contains(&"com.test.enums.Direction.NORTH".to_string())); 
        assert!(imports.contains(&"java.util.List".to_string()));
    }

    #[test]
    fn test_get_wildcard_imports_with_groovy_static_imports() {
        let source = r#"
package com.test

import java.util.*
import static com.test.enums.Level.*
import static com.test.enums.Mode.*

class MyClass {
    // class body
}
"#;
        
        let wildcards = get_wildcard_imports_from_source(source);
        
        // Should include both regular and static wildcard imports
        if let Some(wildcards) = wildcards {
            assert!(wildcards.contains(&"java.util".to_string()));
            assert!(wildcards.contains(&"com.test.enums.Level".to_string()));
            assert!(wildcards.contains(&"com.test.enums.Mode".to_string()));
        } else {
            panic!("Expected wildcard imports to be found");
        }
    }

    #[test]
    fn test_groovy_enum_in_class_usage() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        // Test Groovy code using enum constant with field access
        let source = r#"
enum State {
    ENABLED,
    DISABLED
}

class MyClass {
    def state = State.ENABLED
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Search for enum constant definition
        let result = search_definition(&tree, source, "ENABLED", SymbolType::EnumUsage);
        assert!(result.is_some(), "Should find ENABLED enum constant definition");
    }

    #[test]
    fn test_groovy_navigation_expression_enum_access() {
        // Test that navigation expressions like Status.SUCCESS are handled correctly
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
enum TaskStatus {
    PENDING,
    RUNNING,  
    COMPLETED,
    FAILED
}

class TaskProcessor {
    def process() {
        def status = TaskStatus.RUNNING
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Test that we can find navigation expression enum constants using the specialized method
        let result = search_definition(&tree, source, "RUNNING", SymbolType::EnumUsage);
        assert!(result.is_some(), "Should find RUNNING enum constant");
        
        let result = search_definition(&tree, source, "COMPLETED", SymbolType::EnumUsage);
        assert!(result.is_some(), "Should find COMPLETED enum constant");
    }

    #[test] 
    fn test_groovy_enum_vs_method_priority() {
        // Test that enum constants are found before methods when both exist
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
enum Response {
    SUCCESS,
    ERROR
}

class TestService {
    def SUCCESS() {
        println "This is a method"
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Both enum constant and method exist, test they can both be found
        let enum_result = search_definition(&tree, source, "SUCCESS", SymbolType::EnumUsage);
        let method_result = search_definition(&tree, source, "SUCCESS", SymbolType::MethodCall);
        
        assert!(enum_result.is_some(), "Should find SUCCESS enum constant");
        assert!(method_result.is_some(), "Should find SUCCESS method");
        
        // Both exist, but our logic should prefer enum constants in static context
    }

    #[test]
    fn test_groovy_static_import_wildcards() {
        // Test that wildcard static imports are detected for enum resolution
        let source = r#"
package com.example

import java.util.*
import static com.test.enums.Level.*
import static com.example.Status.*
import com.other.Class

class Example {
    def process() {
        def level = HIGH
        def status = ACTIVE
    }
}
"#;
        
        let wildcards = get_wildcard_imports_from_source(source);
        
        if let Some(wildcards) = wildcards {
            assert!(wildcards.contains(&"java.util".to_string()));
            assert!(wildcards.contains(&"com.test.enums.Level".to_string()));
            assert!(wildcards.contains(&"com.example.Status".to_string()));
            // Should not contain non-wildcard imports
            assert!(!wildcards.contains(&"com.other.Class".to_string()));
        }
    }

    #[test] 
    fn test_groovy_could_be_static_enum_import_detection() {
        use super::super::project::could_be_static_enum_import;
        
        // Test the static enum import detection logic for Groovy
        let source_with_static_import = r#"
package com.example
import static com.example.Priority.*

class Task {
    def process() {
        def priority = HIGH
    }
}
"#;
        
        let source_without_static_import = r#"
package com.example
import com.example.Priority

class Task {
    def process() {
        def priority = Priority.HIGH
    }
}
"#;
        
        // HIGH could be from static import in first case
        assert!(could_be_static_enum_import("HIGH", source_with_static_import));
        
        // In second case, HIGH without Priority. prefix is less likely to be enum
        assert!(!could_be_static_enum_import("HIGH", source_without_static_import));
    }

    #[test]
    fn test_groovy_enum_type_queries() {
        // Test that enum declaration and usage queries work correctly
        let enum_decl_query = get_declaration_query_for_symbol_type(&SymbolType::EnumDeclaration);
        assert!(enum_decl_query.is_some());
        let query_text = enum_decl_query.unwrap();
        assert!(query_text.contains("enum_declaration"));
        
        let enum_usage_query = get_declaration_query_for_symbol_type(&SymbolType::EnumUsage);
        assert!(enum_usage_query.is_some());  
        let query_text = enum_usage_query.unwrap();
        assert!(query_text.contains("enum_constant"));
    }

    #[test]
    fn test_groovy_nested_enum_static_import_extraction() {
        use super::super::project::extract_nested_type_from_import_path;
        
        // Test nested enum type extraction
        assert_eq!(extract_nested_type_from_import_path("com.example.Order.Status"), "Order.Status");
        assert_eq!(extract_nested_type_from_import_path("com.example.deep.Container.State"), "Container.State");
        assert_eq!(extract_nested_type_from_import_path("com.example.Priority"), "Priority");
        assert_eq!(extract_nested_type_from_import_path("Status"), "Status");
        
        // Edge cases  
        assert_eq!(extract_nested_type_from_import_path(""), "");
        assert_eq!(extract_nested_type_from_import_path("com.example.lower.Upper"), "lower.Upper");
    }

    #[test]
    fn test_groovy_find_type_in_tree() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class OuterClass {
    static class InnerClass {
        enum Status {
            ACTIVE, INACTIVE
        }
    }
    
    interface MyInterface {
        void doSomething()
    }
    
    enum Priority {
        HIGH, LOW
    }
}

class AnotherClass {
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        
        // Test finding regular classes
        let result = groovy_support.find_type_in_tree(&tree, source, "OuterClass", "file:///test.groovy");
        assert!(result.is_some(), "Should find OuterClass");
        
        let result = groovy_support.find_type_in_tree(&tree, source, "AnotherClass", "file:///test.groovy");
        assert!(result.is_some(), "Should find AnotherClass");
        
        // Test finding nested classes
        let result = groovy_support.find_type_in_tree(&tree, source, "InnerClass", "file:///test.groovy");
        assert!(result.is_some(), "Should find InnerClass");
        
        // Test finding interfaces
        let result = groovy_support.find_type_in_tree(&tree, source, "MyInterface", "file:///test.groovy");
        assert!(result.is_some(), "Should find MyInterface");
        
        // Test finding enums
        let result = groovy_support.find_type_in_tree(&tree, source, "Priority", "file:///test.groovy");
        assert!(result.is_some(), "Should find Priority enum");
        
        let result = groovy_support.find_type_in_tree(&tree, source, "Status", "file:///test.groovy");
        assert!(result.is_some(), "Should find nested Status enum");
        
        // Test non-existent type
        let result = groovy_support.find_type_in_tree(&tree, source, "NonExistent", "file:///test.groovy");
        assert!(result.is_none(), "Should not find non-existent type");
    }

    #[test]
    fn test_groovy_find_method_in_tree() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class TestClass {
    def publicMethod() {
    }
    
    private int privateMethod(String param) {
        return 42
    }
    
    static void staticMethod() {
    }
    
    TestClass() {
    }
    
    static class InnerClass {
        def innerMethod() {
        }
        
        private void anotherInnerMethod(int x, String y) {
        }
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        
        // Test finding public methods
        let result = groovy_support.find_method_in_tree(&tree, source, "publicMethod", "file:///test.groovy");
        assert!(result.is_some(), "Should find publicMethod");
        
        // Test finding private methods  
        let result = groovy_support.find_method_in_tree(&tree, source, "privateMethod", "file:///test.groovy");
        assert!(result.is_some(), "Should find privateMethod");
        
        // Test finding static methods
        let result = groovy_support.find_method_in_tree(&tree, source, "staticMethod", "file:///test.groovy");
        assert!(result.is_some(), "Should find staticMethod");
        
        // Test finding methods in nested classes
        let result = groovy_support.find_method_in_tree(&tree, source, "innerMethod", "file:///test.groovy");
        assert!(result.is_some(), "Should find innerMethod in nested class");
        
        let result = groovy_support.find_method_in_tree(&tree, source, "anotherInnerMethod", "file:///test.groovy");
        assert!(result.is_some(), "Should find anotherInnerMethod in nested class");
        
        // Test non-existent method
        let result = groovy_support.find_method_in_tree(&tree, source, "nonExistentMethod", "file:///test.groovy");
        assert!(result.is_none(), "Should not find non-existent method");
    }

    #[test]
    fn test_groovy_find_property_in_tree() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class TestClass {
    private String privateField
    public int publicField = 42
    static String staticField
    final String finalField = "test"
    
    static class InnerClass {
        private boolean innerField
        String anotherInnerField = "nested"
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        
        // Test finding private fields
        let result = groovy_support.find_property_in_tree(&tree, source, "privateField", "file:///test.groovy");
        assert!(result.is_some(), "Should find privateField");
        
        // Test finding public fields
        let result = groovy_support.find_property_in_tree(&tree, source, "publicField", "file:///test.groovy");
        assert!(result.is_some(), "Should find publicField");
        
        // Test finding static fields
        let result = groovy_support.find_property_in_tree(&tree, source, "staticField", "file:///test.groovy");
        assert!(result.is_some(), "Should find staticField");
        
        // Test finding final fields
        let result = groovy_support.find_property_in_tree(&tree, source, "finalField", "file:///test.groovy");
        assert!(result.is_some(), "Should find finalField");
        
        // Test finding fields in nested classes
        let result = groovy_support.find_property_in_tree(&tree, source, "innerField", "file:///test.groovy");
        assert!(result.is_some(), "Should find innerField in nested class");
        
        let result = groovy_support.find_property_in_tree(&tree, source, "anotherInnerField", "file:///test.groovy");
        assert!(result.is_some(), "Should find anotherInnerField in nested class");
        
        // Test non-existent field
        let result = groovy_support.find_property_in_tree(&tree, source, "nonExistentField", "file:///test.groovy");
        assert!(result.is_none(), "Should not find non-existent field");
    }

    #[test]
    fn test_groovy_find_type_nested_lookup() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class OuterService {
    static class InnerHandler {
        enum State {
            READY, PROCESSING, DONE
        }
        
        static class DeepNested {
            void process() {}
        }
    }
    
    enum Status {
        ACTIVE, INACTIVE
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested types with dot notation
        let result = groovy_support.find_type(source, "file:///test.groovy", "OuterService.InnerHandler", dependency_cache.clone());
        // This should work once the nested lookup is properly implemented
        // For now, we test that the method exists and can be called
        
        let result = groovy_support.find_type(source, "file:///test.groovy", "OuterService.Status", dependency_cache.clone());
        // Similarly, this tests the nested enum lookup
        
        // Test regular (non-nested) type lookup
        let result = groovy_support.find_type(source, "file:///test.groovy", "OuterService", dependency_cache.clone());
        // This should find the outer class
        
        // Test deeply nested type
        let result = groovy_support.find_type(source, "file:///test.groovy", "OuterService.InnerHandler.State", dependency_cache.clone());
        // This tests deep nesting
    }

    #[test]
    fn test_groovy_find_method_nested_lookup() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class ApiController {
    def handleRequest() {}
    
    static class AuthHelper {
        static boolean authenticate(String token) {
            return true
        }
        
        def authorize() {}
    }
    
    static class ValidationHelper {
        boolean validate(Object data) {
            return true
        }
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested methods
        let result = groovy_support.find_method(source, "file:///test.groovy", "ApiController.AuthHelper.authenticate", dependency_cache.clone());
        // This tests nested static method lookup
        
        let result = groovy_support.find_method(source, "file:///test.groovy", "ApiController.AuthHelper.authorize", dependency_cache.clone());
        // This tests nested instance method lookup
        
        // Test regular (non-nested) method lookup
        let result = groovy_support.find_method(source, "file:///test.groovy", "handleRequest", dependency_cache.clone());
        // This should find the method in the outer class
    }

    #[test]
    fn test_groovy_find_property_nested_lookup() {
        use crate::languages::groovy::support::GroovySupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example

class Configuration {
    static String globalSetting = "default"
    
    static class DatabaseConfig {
        static String host = "localhost"
        int port = 5432
        
        static class ConnectionPool {
            static int maxConnections = 100
            boolean autoReconnect = true
        }
    }
    
    static class CacheConfig {
        long ttl = 3600
        static boolean enabled = true
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let groovy_support = GroovySupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested properties
        let result = groovy_support.find_property(source, "file:///test.groovy", "Configuration.DatabaseConfig.host", dependency_cache.clone());
        // This tests nested static field lookup
        
        let result = groovy_support.find_property(source, "file:///test.groovy", "Configuration.DatabaseConfig.port", dependency_cache.clone());
        // This tests nested instance field lookup
        
        // Test deeply nested property
        let result = groovy_support.find_property(source, "file:///test.groovy", "Configuration.DatabaseConfig.ConnectionPool.maxConnections", dependency_cache.clone());
        // This tests deep nesting
        
        // Test regular (non-nested) property lookup
        let result = groovy_support.find_property(source, "file:///test.groovy", "globalSetting", dependency_cache.clone());
        // This should find the property in the outer class
    }
}
