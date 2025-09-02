use std::sync::Arc;

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{dependency_cache::DependencyCache, symbols::SymbolType, utils::path_to_file_uri},
    languages::LanguageSupport,
};

use super::utils::{
    extract_package_from_source, prepare_symbol_lookup_key_with_wildcard_support,
    resolve_symbol_with_imports, search_definition_in_project,
};

#[tracing::instrument(skip_all)]
pub async fn find_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    
    // FIRST: Check for nested enum access patterns (e.g., Product.Status.DISABLING)
    if let Some(parent) = usage_node.parent() {
        if parent.kind() == "navigation_expression" {
            if let Some(enum_type_node) = parent.child(0) {
                if let Some(enum_type_name) = resolve_nested_enum_type(source, &enum_type_node) {
                    if enum_type_name.contains('.') {
                        return find_nested_enum_using_regular_resolution(
                            source,
                            file_uri,
                            &enum_type_name,
                            symbol_text,
                            dependency_cache.clone(),
                            language_support,
                        ).await;
                    }
                }
            }
        }
    }
    
    // SECOND: Try regular symbol resolution
    let regular_search_result = try_regular_symbol_search(
        source, 
        file_uri, 
        usage_node, 
        dependency_cache.clone(), 
        language_support
    ).await;
    
    if regular_search_result.is_some() {
        return regular_search_result;
    }
    
    // If regular search fails and this could be a static enum import, try enum strategies
    if could_be_static_enum_import(symbol_text, source) {
        // Try project-level first
        if let Some(enum_location) = find_enum_constant_in_project(
            source,
            file_uri,
            usage_node,
            dependency_cache.clone(),
            language_support,
        )
        .await
        {
            return Some(enum_location);
        }
        
        // Try workspace-level if project fails
        if let Some(enum_location) = find_enum_constant_in_workspace(
            source,
            file_uri,
            usage_node,
            dependency_cache.clone(),
            language_support,
        )
        .await
        {
            return Some(enum_location);
        }
        
        // Try external dependencies if workspace fails
        if let Some(enum_location) = find_enum_constant_in_external(
            source,
            file_uri,
            usage_node,
            dependency_cache.clone(),
            language_support,
        )
        .await
        {
            return Some(enum_location);
        }
    }

    None
}

/// Handle enum constant lookup specially - extract enum type and find the constant within it
#[tracing::instrument(skip_all)]
async fn find_enum_constant_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let constant_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Check if this is a static import case or navigation expression case  
    let (enum_type_name, enum_type_node) = if let Some(navigation_expr) =
        usage_node.parent().and_then(|p| {
            if p.kind() == "navigation_suffix" { 
                p.parent().and_then(|pp| if pp.kind() == "navigation_expression" { Some(pp) } else { None })
            } else { 
                None 
            }
        }) {
        // Case 1: Color.RED (navigation expression)
        let enum_type_node = navigation_expr.child(0)?;
        let enum_type_name = resolve_nested_enum_type(source, &enum_type_node)?;
        (enum_type_name, Some(enum_type_node))
    } else {
        // Case 2: RED (static import)
        let enum_type_name = extract_enum_type_from_static_import(source, &constant_name)?;
        (enum_type_name, None)
    };

    // Use the existing wildcard resolution to find the enum type
    let enum_symbol_key = if let Some(enum_node) = enum_type_node {
        prepare_symbol_lookup_key_with_wildcard_support(
            &enum_node,
            source,
            file_uri,
            None,
            &dependency_cache,
        )
    } else {
        // For static imports, we can't use wildcard support with a node,
        // so we'll handle it in the fallback section
        None
    };

    let (project_root, enum_fqn) = if let Some(key) = enum_symbol_key {
        key
    } else {
        // Fallback: construct FQN for enum type
        let project_root = crate::core::utils::uri_to_path(file_uri)
            .and_then(|path| crate::core::utils::find_project_root(&path))?;

        let enum_fqn = if let Some(resolved_fqn) = resolve_symbol_with_imports(&enum_type_name, source, &dependency_cache) {
            resolved_fqn
        } else if let Some(package) = extract_package_from_source(source) {
            if !package.is_empty() {
                let fqn = format!("{}.{}", package, enum_type_name);
                fqn
            } else {
                enum_type_name.clone()
            }
        } else {
            enum_type_name.clone()
        };

        (project_root, enum_fqn)
    };
    

    // Handle nested enum resolution (e.g., Foo.Status)
    if enum_type_name.contains('.') {
        return find_nested_enum_constant(
            &project_root,
            &enum_type_name,
            &constant_name,
            dependency_cache,
        )
        .await;
    }

    // Find the enum type definition (for top-level enums)
    if let Some(target_file_path) = dependency_cache.find_symbol_sync(&project_root, &enum_fqn) {
        let target_file_uri = path_to_file_uri(&target_file_path)?;
        let target_tree = crate::core::utils::uri_to_tree(&target_file_uri)?;
        let target_source = std::fs::read_to_string(&target_file_path).ok()?;

        // Find the specific enum constant within the enum definition
        return find_enum_constant_in_enum_definition(&target_tree, &target_source, &constant_name, &target_file_uri);
    }

    None
}

/// Resolve nested enum type from navigation expression (e.g., Foo.Status -> fully qualified path)
#[tracing::instrument(skip_all)]
pub fn resolve_nested_enum_type(source: &str, enum_type_node: &Node<'_>) -> Option<String> {
    // For simple identifier, return as-is
    if enum_type_node.kind() == "simple_identifier" || enum_type_node.kind() == "type_identifier" {
        return enum_type_node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
    }
    
    // For nested navigation expression (Foo.Status), extract the full path
    if enum_type_node.kind() == "navigation_expression" {
        return enum_type_node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
    }
    
    // Fallback: extract the full text
    enum_type_node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
}

/// Find nested enum constant (e.g., Foo.Status.ACTIVE)
#[tracing::instrument(skip_all)]
async fn find_nested_enum_constant(
    project_root: &std::path::PathBuf,
    nested_enum_type: &str,
    constant_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // Split nested path: "Foo.Status" -> ("Foo", "Status")
    let parts: Vec<&str> = nested_enum_type.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    
    let outer_class = parts[0];
    let inner_enum_path = &parts[1..].join(".");
    
    // Find the outer class
    if let Some(target_file_path) = dependency_cache.find_symbol_sync(project_root, outer_class) {
        let target_file_uri = path_to_file_uri(&target_file_path)?;
        let target_tree = crate::core::utils::uri_to_tree(&target_file_uri)?;
        let target_source = std::fs::read_to_string(&target_file_path).ok()?;
        
        // Find the inner enum within the outer class
        return find_inner_enum_constant(
            &target_tree,
            &target_source,
            inner_enum_path,
            constant_name,
            &target_file_uri,
        );
    }
    
    None
}

/// Find enum constant within inner enum of a class
#[tracing::instrument(skip_all)]
fn find_inner_enum_constant(
    tree: &tree_sitter::Tree,
    source: &str,
    inner_enum_path: &str,
    constant_name: &str,
    file_uri: &str,
) -> Option<Location> {
    use super::utils::get_or_create_query;
    use tree_sitter::{QueryCursor, StreamingIterator};
    
    // Find the inner enum declaration by name (Kotlin uses class_declaration for enums)
    let enum_query_text = r#"(class_declaration (type_identifier) @enum_name)"#;
    let enum_query = get_or_create_query(enum_query_text).ok()?;
    
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&enum_query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(enum_name) = capture.node.utf8_text(source.as_bytes()) {
                if enum_name == inner_enum_path {
                    // Found the inner enum, now search for the constant within it
                    let enum_node = capture.node.parent()?; // Get the full class_declaration node
                    return find_enum_constant_in_node(&enum_node, source, constant_name, file_uri);
                }
            }
        }
    }
    
    None
}

/// Recursively find enum constant in a node (supports nested structures)
#[tracing::instrument(skip_all)]
fn find_enum_constant_in_node(
    node: &Node<'_>,
    source: &str,
    constant_name: &str,
    file_uri: &str,
) -> Option<Location> {
    use super::utils::get_or_create_query;
    use crate::core::utils::node_to_lsp_location;
    use tree_sitter::{QueryCursor, StreamingIterator};

    let query_text = r#"(enum_entry (simple_identifier) @constant_name)"#;
    let query = get_or_create_query(query_text).ok()?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, *node, source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == constant_name {
                    return node_to_lsp_location(&capture.node, file_uri);
                }
            }
        }
    }

    None
}

/// Find a specific enum constant within an enum definition (handles nested enums)
#[tracing::instrument(skip_all)]
fn find_enum_constant_in_enum_definition(
    tree: &tree_sitter::Tree,
    source: &str,
    constant_name: &str,
    file_uri: &str,
) -> Option<Location> {
    find_enum_constant_in_node(&tree.root_node(), source, constant_name, file_uri)
}

/// Extract enum type name from static import statements for a given constant
#[tracing::instrument(skip_all)]
fn extract_enum_type_from_static_import(source: &str, _constant_name: &str) -> Option<String> {
    use super::utils::get_or_create_query;
    use tree_sitter::{Parser, QueryCursor, StreamingIterator};

    // Create a tree for this source
    let mut parser = Parser::new();
    let language = tree_sitter_kotlin::language();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    // Look for static import statements with asterisk (wildcard imports)
    let query_text = r#"
        (import_header 
            (identifier) @import_path 
            (wildcard_import))
    "#;

    let query = get_or_create_query(query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    // Collect all static import paths with nested support
    let mut static_imports = Vec::new();
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(import_path) = capture.node.utf8_text(source.as_bytes()) {
                // For nested enums like "com.example.Foo.Status", extract "Foo.Status"
                let nested_type = extract_nested_type_from_import_path(import_path);
                static_imports.push(nested_type);
            }
        }
    }

    // Return the first static import that looks like an enum
    for class_name in &static_imports {
        if class_name.ends_with("Enum") || class_name.contains("Status") || class_name.contains("Type") {
            return Some(class_name.clone());
        }
    }

    // If no enum-like class found, return the first static import
    static_imports.first().cloned()
}

/// Extract nested type from import path (e.g., "com.example.Foo.Status" -> "Foo.Status")
#[tracing::instrument(skip_all)]
pub fn extract_nested_type_from_import_path(import_path: &str) -> String {
    let parts: Vec<&str> = import_path.split('.').collect();
    
    if parts.len() >= 2 {
        let second_last = parts[parts.len() - 2];
        let last = parts[parts.len() - 1];
        
        // Special case for "lower.Upper" pattern in longer paths
        if parts.len() >= 4 && second_last.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) 
            && last.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            return format!("{}.{}", second_last, last);
        }
        
        // If both second_last and last are capitalized, return both
        if second_last.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) 
            && last.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            return format!("{}.{}", second_last, last);
        }
    }
    
    // Fallback: return just the last part
    parts.last().map_or("", |v| v).to_string()
}

/// Extract full FQN from static import statements for a given constant
#[tracing::instrument(skip_all)]
fn extract_full_fqn_from_static_import(source: &str, _constant_name: &str) -> Option<String> {
    use super::utils::get_or_create_query;
    use tree_sitter::{Parser, QueryCursor, StreamingIterator};

    // Create a tree for this source
    let mut parser = Parser::new();
    let language = tree_sitter_kotlin::language();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    // Look for static import statements with asterisk (wildcard imports)
    let query_text = r#"
        (import_header 
            (identifier) @import_path 
            (wildcard_import))
    "#;

    let query = get_or_create_query(query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    // Collect all static import paths with both full path and class name
    let mut static_imports = Vec::new();
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(import_path) = capture.node.utf8_text(source.as_bytes()) {
                if let Some(class_name) = import_path.split('.').last() {
                    static_imports.push((import_path.to_string(), class_name.to_string()));
                }
            }
        }
    }

    // Return the full path of the first static import that looks like an enum
    for (full_path, class_name) in &static_imports {
        if class_name.ends_with("Enum") || class_name.contains("Status") || class_name.contains("Type") {
            return Some(full_path.clone());
        }
    }

    // If no enum-like class found, return the full path of the first static import
    static_imports.first().map(|(full_path, _)| full_path.clone())
}

/// Handle enum constant lookup in workspace (different projects) 
#[tracing::instrument(skip_all)]
async fn find_enum_constant_in_workspace(
    source: &str,
    _file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let constant_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Extract full FQN from static import statements
    let resolved_fqn = if let Some(full_fqn) = extract_full_fqn_from_static_import(source, &constant_name) {
        full_fqn
    } else {
        // Fallback: extract just the enum type name and try to resolve it
        let enum_type_name = extract_enum_type_from_static_import(source, &constant_name)?;
        
        if let Some(resolved_fqn) = resolve_symbol_with_imports(&enum_type_name, source, &dependency_cache) {
            resolved_fqn
        } else {
            enum_type_name
        }
    };
    

    // Get unique project roots from the symbol index
    let mut project_roots = std::collections::HashSet::new();
    
    for entry in dependency_cache.symbol_index.iter() {
        let ((project_root, _), _) = (entry.key(), entry.value());
        project_roots.insert(project_root.clone());
    }
    
    // If symbol index is empty or has no projects, try alternative approach
    if project_roots.is_empty() {
        for entry in dependency_cache.project_metadata.iter() {
            project_roots.insert(entry.key().clone());
        }
    }
    
    // Search in each unique project
    for project_root in project_roots {
        if let Some(target_file_path) = dependency_cache.find_symbol(&project_root, &resolved_fqn).await {
            let target_file_uri = crate::core::utils::path_to_file_uri(&target_file_path)?;
            let target_tree = crate::core::utils::uri_to_tree(&target_file_uri)?;
            let target_source = std::fs::read_to_string(&target_file_path).ok()?;

            // Find the specific enum constant within the enum definition
            return find_enum_constant_in_enum_definition(
                &target_tree,
                &target_source,
                &constant_name,
                &target_file_uri,
            );
        }
    }

    None
}

/// Handle enum constant lookup in external dependencies (JAR files, etc.)
#[tracing::instrument(skip_all)]
async fn find_enum_constant_in_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let constant_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Extract full FQN from static import statements
    let resolved_fqn = if let Some(full_fqn) = extract_full_fqn_from_static_import(source, &constant_name) {
        full_fqn
    } else {
        // Fallback: extract just the enum type name and try to resolve it
        let enum_type_name = extract_enum_type_from_static_import(source, &constant_name)?;
        
        if let Some(resolved_fqn) = resolve_symbol_with_imports(&enum_type_name, source, &dependency_cache) {
            resolved_fqn
        } else {
            enum_type_name
        }
    };

    let current_project = crate::core::utils::uri_to_path(file_uri)
        .and_then(|path| crate::core::utils::find_project_root(&path))?;

    // Try to find in external dependencies
    if let Some(source_info) = dependency_cache
        .find_external_symbol_with_lazy_parsing(&current_project, &resolved_fqn)
        .await
    {
        let tree = source_info.get_tree().ok()?;
        let content = source_info.get_content().ok()?;
        let target_file_uri = crate::core::jar_utils::get_uri(&source_info)?;

        // Find the specific enum constant within the enum definition
        return find_enum_constant_in_enum_definition(
            &tree,
            &content,
            &constant_name,
            &target_file_uri,
        );
    }

    None
}

/// Check if a symbol could potentially be a static enum import constant
#[tracing::instrument(skip_all)]
pub fn could_be_static_enum_import(symbol_text: &str, source: &str) -> bool {
    // Must be ALL_CAPS to be considered an enum constant
    if !symbol_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) {
        return false;
    }
    
    // Check if there are any static imports in this file
    has_static_imports_in_source(source)
}

/// Check if the source has any static import statements
#[tracing::instrument(skip_all)]
fn has_static_imports_in_source(source: &str) -> bool {
    use super::utils::get_or_create_query;
    use tree_sitter::{Parser, QueryCursor, StreamingIterator};
    
    let mut parser = Parser::new();
    let language = tree_sitter_kotlin::language();
    if parser.set_language(&language).is_err() {
        return false;
    }
    
    if let Some(tree) = parser.parse(source, None) {
        let query_text = r#"
            (import_header 
                (identifier) @import_path 
                (wildcard_import))
        "#;
        
        if let Ok(query) = get_or_create_query(query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            matches.next().is_some()
        } else {
            false
        }
    } else {
        false
    }
}

/// Try regular symbol search (the original logic)
#[tracing::instrument(skip_all)]
async fn try_regular_symbol_search(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    
    // Check if this is an enum constant access - handle it specially
    if let Ok(symbol_type) = language_support.determine_symbol_type_from_context(
        &crate::core::utils::uri_to_tree(file_uri)?, 
        usage_node, 
        source
    ) {
        if symbol_type == SymbolType::EnumUsage {
            if let Some(enum_location) = find_enum_constant_in_project(
                source, file_uri, usage_node, dependency_cache.clone(), language_support
            ).await {
                return Some(enum_location);
            }
        }
    }

    let symbol_key = prepare_symbol_lookup_key_with_wildcard_support(
        usage_node,
        source,
        file_uri,
        None,
        &dependency_cache,
    );

    let (project_root, fqn) = if let Some(key) = symbol_key {
        key
    } else {
        let project_root = crate::core::utils::uri_to_path(file_uri)
            .and_then(|path| crate::core::utils::find_project_root(&path))?;

        // Try to resolve FQN using imports first, then fallback to current package
        // But avoid this for enum constants that are likely static imports - let enum-specific logic handle them
        let fqn = if could_be_static_enum_import(&symbol_name, source) {
            // For potential static enum imports, don't try to resolve the constant name directly
            // Let the enum-specific logic handle this properly
            return None;
        } else if let Some(resolved_fqn) =
            resolve_symbol_with_imports(&symbol_name, source, &dependency_cache)
        {
            resolved_fqn
        } else if let Some(package) = extract_package_from_source(source) {
            if !package.is_empty() {
                format!("{}.{}", package, symbol_name)
            } else {
                symbol_name.clone()
            }
        } else {
            symbol_name.clone()
        };

        (project_root, fqn)
    };

    let file_location = dependency_cache.find_symbol(&project_root, &fqn).await?;
    let other_uri = path_to_file_uri(&file_location)?;

    if file_uri == &other_uri {
        // Local definitions should be handled by find_local function
        return None;
    }

    search_definition_in_project(file_uri, source, usage_node, &other_uri, language_support)
}

/// Find nested enum using regular go-to-definition chain
#[tracing::instrument(skip_all)]
pub async fn find_nested_enum_using_regular_resolution(
    source: &str,
    file_uri: &str,
    nested_path: &str,
    target_symbol: &str,
    dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    find_nested_symbol_generic(
        source,
        file_uri,
        nested_path,
        target_symbol,
        dependency_cache,
        |tree, source, inner_path, target_symbol, file_uri| {
            find_inner_enum_constant(tree, source, inner_path, target_symbol, file_uri)
        },
    ).await
}

/// Generic nested symbol resolution: Find outer class, then delegate to specific search
#[tracing::instrument(skip_all)]
async fn find_nested_symbol_generic<F>(
    source: &str,
    file_uri: &str,
    nested_path: &str,
    target_symbol: &str,
    dependency_cache: Arc<DependencyCache>,
    inner_search_fn: F,
) -> Option<Location>
where
    F: FnOnce(&tree_sitter::Tree, &str, &str, &str, &str) -> Option<Location>,
{
    // Split nested path: "Foo.Status" -> ("Foo", "Status")
    let parts: Vec<&str> = nested_path.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    
    let outer_class = parts[0];
    let inner_path = &parts[1..].join(".");
    
    // Step 1: Find the outer class (reusable for all nested access patterns)
    let outer_class_location = find_outer_class_with_multi_level_search(
        source,
        file_uri,
        outer_class,
        dependency_cache.clone(),
    ).await;
    
    if let Some(target_file_path) = outer_class_location {
        let target_file_uri = path_to_file_uri(&target_file_path)?;
        let target_tree = crate::core::utils::uri_to_tree(&target_file_uri)?;
        let target_source = std::fs::read_to_string(&target_file_path).ok()?;
        
        // Step 2: Search within the outer class (specific to the symbol type)
        return inner_search_fn(
            &target_tree,
            &target_source,
            inner_path,
            target_symbol,
            &target_file_uri,
        );
    }
    
    None
}

/// Use multi-level search like regular go-to-definition (project -> workspace -> external)
#[tracing::instrument(skip_all)]
async fn find_outer_class_with_multi_level_search(
    source: &str,
    file_uri: &str,
    outer_class: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<std::path::PathBuf> {
    // Resolve the outer class FQN first
    let outer_class_fqn = if let Some(resolved_fqn) = 
        resolve_symbol_with_imports(outer_class, source, &dependency_cache) {
        resolved_fqn
    } else {
        if let Some(package) = extract_package_from_source(source) {
            if !package.is_empty() {
                format!("{}.{}", package, outer_class)
            } else {
                outer_class.to_string()
            }
        } else {
            outer_class.to_string()
        }
    };

    let project_root = crate::core::utils::uri_to_path(file_uri)
        .and_then(|path| crate::core::utils::find_project_root(&path))?;

    // Level 1: Try current project
    if let Some(path) = dependency_cache.find_symbol(&project_root, &outer_class_fqn).await {
        return Some(path);
    }

    // Level 2: Try workspace (other projects) - search all projects
    for entry in dependency_cache.symbol_index.iter() {
        let ((other_project_root, _), _) = (entry.key(), entry.value());
        if other_project_root != &project_root {
            if let Some(path) = dependency_cache.find_symbol(other_project_root, &outer_class_fqn).await {
                return Some(path);
            }
        }
    }

    // Level 3: Try external dependencies 
    if let Some(source_info) = dependency_cache
        .find_external_symbol_with_lazy_parsing(&project_root, &outer_class_fqn)
        .await
    {
        return Some(source_info.source_path.clone());
    }

    None
}

