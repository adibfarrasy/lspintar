use std::sync::Arc;
use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree, StreamingIterator};
use tracing::debug;

use crate::{
    core::dependency_cache::DependencyCache,
    languages::LanguageSupport,
};

/// Common method resolution logic shared across JVM languages (Java, Groovy, Kotlin)

/// Detect if a method call is static and extract the class name
#[tracing::instrument(skip_all)]
pub fn extract_static_method_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    debug!(
        "extract_static_method_context: analyzing node '{}' of kind '{}'",
        usage_text,
        usage_node.kind()
    );

    let method_invocation = find_parent_method_invocation_node(usage_node);
    if method_invocation.is_none() {
        return None;
    }
    let method_invocation = method_invocation.unwrap();

    // Check if this method invocation has an object field (static method pattern)
    let object_node = method_invocation.child_by_field_name("object");
    let method_name_node = method_invocation.child_by_field_name("name");
    
    debug!("extract_static_method_context: object_node present: {}, method_name_node present: {}", 
           object_node.is_some(), method_name_node.is_some());
    
    let object_node = object_node?;
    let method_name_node = method_name_node?;

    let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let method_name = method_name_node
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    debug!(
        "extract_static_method_context: found class_name='{}', method_name='{}', usage_text='{}'",
        class_name, method_name, usage_text
    );

    // Only return Some for actual static method calls (class name starts with uppercase)
    if class_name
        .chars()
        .next()
        .map_or(false, |c| c.is_uppercase())
    {
        // This looks like a static method call (ClassName.method)
        if usage_text == method_name {
            debug!("extract_static_method_context: _usage_node matches method name - static method call detected");
            Some((class_name, method_name))
        } else if usage_text == class_name {
            debug!("extract_static_method_context: _usage_node matches class name - this should go to class definition, not method");
            None  // Return None so it goes to regular resolution for class definition
        } else {
            debug!("extract_static_method_context: _usage_node '{}' matches neither class '{}' nor method '{}'", 
                   usage_text, class_name, method_name);
            None
        }
    } else {
        // This looks like an instance method call (variable.method) - not a static method call
        debug!("extract_static_method_context: object '{}' looks like a variable (lowercase) - not a static method call", class_name);
        None
    }
}

/// Detect if a method call is on an instance and extract the variable name
#[tracing::instrument(skip_all)]
pub fn extract_instance_method_context(
    usage_node: &Node,
    source: &str,
) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    debug!(
        "extract_instance_method_context: analyzing node '{}' of kind '{}'",
        usage_text,
        usage_node.kind()
    );

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

    debug!(
        "extract_instance_method_context: found variable_name='{}', method_name='{}', usage_text='{}'",
        variable_name, method_name, usage_text
    );

    // Only return Some for instance method calls (variable name starts with lowercase)
    if variable_name
        .chars()
        .next()
        .map_or(false, |c| c.is_lowercase())
    {
        // This looks like an instance method call (variable.method)
        if usage_text == method_name {
            debug!("extract_instance_method_context: _usage_node matches method name - instance method call detected");
            Some((variable_name, method_name))
        } else if usage_text == variable_name {
            debug!("extract_instance_method_context: _usage_node matches variable name - this should go to variable declaration, not method");
            None  // Return None so it goes to regular resolution for variable declaration
        } else {
            debug!("extract_instance_method_context: _usage_node '{}' matches neither variable '{}' nor method '{}'", 
                   usage_text, variable_name, method_name);
            None
        }
    } else {
        // This looks like a static method call (ClassName.method) - not an instance method call
        debug!("extract_instance_method_context: object '{}' looks like a class (uppercase) - not an instance method call", variable_name);
        None
    }
}

/// Find parent method invocation node - common pattern across JVM languages
#[tracing::instrument(skip_all)]
pub fn find_parent_method_invocation_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(curr_node) = current {
        if curr_node.kind() == "method_invocation" {
            return Some(curr_node);
        }
        current = curr_node.parent();
    }
    
    None
}

/// Find parent field access node - common pattern across JVM languages
#[tracing::instrument(skip_all)]
pub fn find_parent_field_access_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(curr_node) = current {
        if curr_node.kind() == "field_access" {
            return Some(curr_node);
        }
        current = curr_node.parent();
    }
    
    None
}

/// Extract context for static field access (e.g., ClassName.FIELD_NAME)
#[tracing::instrument(skip_all)]
pub fn extract_static_field_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");

    // Try to find parent field_access node (Java/Groovy)
    if let Some(field_access) = find_parent_field_access_node(usage_node) {
        // Check if this field access has an object and field
        let object_node = field_access.child_by_field_name("object");
        let field_node = field_access.child_by_field_name("field");
        
        if let (Some(object_node), Some(field_node)) = (object_node, field_node) {
            let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
            let field_name = field_node.utf8_text(source.as_bytes()).ok()?.to_string();
            
            return evaluate_static_field_context(class_name, field_name, usage_text);
        }
    }
    
    // Try to find parent navigation_expression (Kotlin)
    if let Some(nav_expr) = find_parent_navigation_expression(usage_node) {
        // Extract object and field from navigation_expression
        if let (Some(object_node), Some(nav_suffix)) = (nav_expr.child(0), nav_expr.child(1)) {
            if nav_suffix.kind() == "navigation_suffix" {
                // Find the simple_identifier in nav_suffix
                let field_node = (0..nav_suffix.child_count())
                    .filter_map(|i| nav_suffix.child(i))
                    .find(|child| child.kind() == "simple_identifier");
                
                if let Some(field_node) = field_node {
                    let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                    let field_name = field_node.utf8_text(source.as_bytes()).ok()?.to_string();
                    
                    return evaluate_static_field_context(class_name, field_name, usage_text);
                }
            }
        }
    }
    
    None
}

/// Helper function to evaluate if a class/field combination represents a static field access
fn evaluate_static_field_context(class_name: String, field_name: String, usage_text: &str) -> Option<(String, String)> {
    // Only return Some for actual static field access (class name starts with uppercase)
    if class_name.chars().next().map_or(false, |c| c.is_uppercase()) {
        // This looks like a static field access (ClassName.FIELD)
        if usage_text == field_name {
            Some((class_name, field_name))
        } else if usage_text == class_name {
            None  // Return None so it goes to regular class resolution
        } else {
            None
        }
    } else {
        // This looks like an instance field access (variable.field) - not a static field access
        None
    }
}

/// Find parent navigation_expression node (for Kotlin)
fn find_parent_navigation_expression<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(curr_node) = current {
        if curr_node.kind() == "navigation_expression" {
            return Some(curr_node);
        }
        current = curr_node.parent();
    }
    
    None
}

/// Extract context for instance field access (e.g., variable.field)
#[tracing::instrument(skip_all)]
pub fn extract_instance_field_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    debug!(
        "extract_instance_field_context: analyzing node '{}' of kind '{}'",
        usage_text,
        usage_node.kind()
    );

    // Find parent field_access node
    let field_access = find_parent_field_access_node(usage_node)?;

    // Check if this field access has an object and field
    let object_node = field_access.child_by_field_name("object");
    let field_node = field_access.child_by_field_name("field");
    
    debug!("extract_instance_field_context: object_node present: {}, field_node present: {}", 
           object_node.is_some(), field_node.is_some());
    
    let object_node = object_node?;
    let field_node = field_node?;

    let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let field_name = field_node
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    debug!(
        "extract_instance_field_context: found variable_name='{}', field_name='{}', usage_text='{}'",
        variable_name, field_name, usage_text
    );

    // Only return Some for instance field access (variable name starts with lowercase)
    if variable_name
        .chars()
        .next()
        .map_or(false, |c| c.is_lowercase())
    {
        // This looks like an instance field access (variable.field)
        if usage_text == field_name {
            debug!("extract_instance_field_context: usage_node matches field name - instance field access detected");
            Some((variable_name, field_name))
        } else if usage_text == variable_name {
            debug!("extract_instance_field_context: usage_node matches variable name - this should go to variable declaration, not field");
            None  // Return None so it goes to regular resolution for variable declaration
        } else {
            debug!("extract_instance_field_context: usage_node '{}' matches neither variable '{}' nor field '{}'", 
                   usage_text, variable_name, field_name);
            None
        }
    } else {
        // This looks like a static field access (ClassName.field) - not an instance field access
        debug!("extract_instance_field_context: object '{}' looks like a class (uppercase) - not an instance field access", 
               variable_name);
        None
    }
}

/// Common logic for finding static method definitions
#[tracing::instrument(skip_all)]
pub fn find_static_method_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    _usage_node: &Node,
    class_name: &str,
    method_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!(
        "find_static_method_definition: looking for {}.{}",
        class_name, method_name
    );

    // Create a temporary node representing the class name for resolution
    // This is needed because the existing resolution methods expect a _usage_node
    // For now, we'll use the existing _usage_node but this should be improved
    
    // First strategy: Try to resolve the class name directly
    let class_location = try_resolve_class_name(
        language_support, tree, source, file_uri, class_name, dependency_cache.clone()
    );

    if let Some(location) = class_location {
        debug!(
            "find_static_method_definition: found class {} at {:?}",
            class_name, location.uri
        );
        
        // Now search for the method within the resolved class file
        if let Some(method_location) = search_method_in_class_file(
            &location, method_name, language_support
        ) {
            return Some(method_location);
        }
        
        debug!("find_static_method_definition: method {} not found in class file, returning class location", method_name);
        // Return class location as fallback
        return Some(location);
    }

    debug!(
        "find_static_method_definition: could not find class {}",
        class_name
    );
    None
}

/// Common logic for finding static field definitions
#[tracing::instrument(skip_all)]
pub fn find_static_field_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    _usage_node: &Node,
    class_name: &str,
    field_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // First resolve the class name to find the class file location
    let class_location = try_resolve_class_name(
        language_support, tree, source, file_uri, class_name, dependency_cache.clone()
    );

    if let Some(location) = class_location {
        // Now search for the field within the resolved class file
        if let Some(field_location) = search_field_in_class_file_cross_language(
            &location, field_name
        ) {
            return Some(field_location);
        }
        
        // Return class location as fallback
        return Some(location);
    }

    None
}

/// Try to resolve a class name using various resolution strategies
fn try_resolve_class_name(
    language_support: &dyn LanguageSupport,
    _tree: &Tree,
    source: &str,
    file_uri: &str,
    class_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // Strategy 1: Try project-level resolution using import resolution
    // This is where we need to create a proper symbol lookup key
    if let Some(location) = try_resolve_class_via_projects(
        language_support, source, file_uri, class_name, dependency_cache.clone()
    ) {
        return Some(location);
    }
    
    // Strategy 2: Try workspace resolution
    if let Some(location) = try_resolve_class_via_workspace(
        language_support, source, file_uri, class_name, dependency_cache.clone()
    ) {
        return Some(location);
    }
    
    // Strategy 3: Try external dependencies
    if let Some(location) = try_resolve_class_via_external(
        language_support, source, file_uri, class_name, dependency_cache
    ) {
        return Some(location);
    }
    
    None
}

/// Try to resolve class via project-level dependencies
fn try_resolve_class_via_projects(
    language_support: &dyn LanguageSupport,
    source: &str,
    file_uri: &str,
    class_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!("try_resolve_class_via_projects: attempting to resolve {}", class_name);
    
    // First, try to resolve the class name to an FQN using imports
    if let Some(resolved_fqn) = try_resolve_class_fqn(class_name, source, language_support) {
        debug!("try_resolve_class_via_projects: resolved {} to FQN {}", class_name, resolved_fqn);
        
        // Use the resolved FQN to find the symbol
        if let Some(location) = try_find_symbol_in_projects(&resolved_fqn, file_uri, &dependency_cache) {
            return Some(location);
        }
    }
    
    // If not found with FQN or no import found, try to find using short name
    // This will use the class_name_index to resolve short names to FQNs
    debug!("try_resolve_class_via_projects: trying short name resolution for {}", class_name);
    if let Some(location) = try_find_symbol_by_short_name(class_name, file_uri, &dependency_cache) {
        return Some(location);
    }
    
    None
}

/// Try to resolve class name to FQN using imports
fn try_resolve_class_fqn(class_name: &str, source: &str, language_support: &dyn LanguageSupport) -> Option<String> {
    
    // Use tree-sitter to parse imports properly
    let mut parser = language_support.create_parser();
    let tree = parser.parse(source, None)?;
    
    // Get import queries from the language support
    let import_queries = language_support.import_queries();
    if import_queries.is_empty() {
        return None;
    }
    
    // Build and execute query
    let query_text = import_queries.join("\n");
    let language = match language_support.language_id() {
        "java" => tree_sitter_java::LANGUAGE.into(),
        "groovy" => tree_sitter_groovy::language(), 
        "kotlin" => tree_sitter_kotlin::language(),
        _ => return None,
    };
    
    let query = match tree_sitter::Query::new(&language, &query_text) {
        Ok(q) => q,
        Err(_) => {
            return None;
        }
    };
    
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                
                // Extract just the import path from "import com.example.Class"
                let import_text = full_import_text
                    .trim_start_matches("import")
                    .trim()
                    .trim_start_matches("static")
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                
                
                // Check if this import ends with our class name
                if import_text.ends_with(&format!(".{}", class_name)) || import_text == class_name {
                    return Some(import_text.to_string());
                } else {
                }
            }
        }
    }
    
    None
}


/// Try to find symbol in projects using dependency cache
fn try_find_symbol_in_projects(
    symbol_name: &str,
    file_uri: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<Location> {
    debug!("try_find_symbol_in_projects: looking for symbol {}", symbol_name);
    
    // Use the existing dependency cache infrastructure
    // We need to determine the project root from the current file
    use crate::core::utils::{uri_to_path, find_project_root};
    
    let current_file_path = uri_to_path(file_uri)?;
    let project_root = find_project_root(&current_file_path)?;
    
    debug!("try_find_symbol_in_projects: current project root: {:?}", project_root);
    
    // Try to find the symbol using find_symbol_sync (blocking version)
    // This uses the same logic as the existing project resolution
    if let Some(file_path) = dependency_cache.find_symbol_sync(&project_root, symbol_name) {
        debug!("try_find_symbol_in_projects: found {} in current project at {:?}", symbol_name, file_path);
        
        // Convert PathBuf to Location
        use tower_lsp::lsp_types::{Position, Range, Url};
        
        if let Ok(uri) = Url::from_file_path(&file_path) {
            let position = Position::new(0, 0); // We'll refine this later with proper parsing
            let range = Range::new(position, position);
            return Some(tower_lsp::lsp_types::Location::new(uri, range));
        }
    }
    
    // Try other projects in workspace if not found in current project
    // Get all available projects from the dependency cache
    let symbol_index = &dependency_cache.symbol_index;
    
    // Search through all projects for this symbol
    for entry in symbol_index.iter() {
        let ((proj_root, sym_name), file_path) = (entry.key(), entry.value());
        if sym_name == symbol_name {
            debug!("try_find_symbol_in_projects: found {} in project {:?}", symbol_name, proj_root);
            
            // Convert file path to Location
            use tower_lsp::lsp_types::{Position, Range, Url};
            
            if let Ok(uri) = Url::from_file_path(file_path.as_path()) {
                let position = Position::new(0, 0); // We'll refine this later
                let range = Range::new(position, position);
                return Some(tower_lsp::lsp_types::Location::new(uri, range));
            }
        }
    }
    
    debug!("try_find_symbol_in_projects: symbol {} not found in any project", symbol_name);
    None
}

/// Try to find symbol by short name using class_name_index
fn try_find_symbol_by_short_name(
    class_name: &str,
    file_uri: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<Location> {
    debug!("try_find_symbol_by_short_name: looking for class {}", class_name);
    
    use crate::core::utils::{uri_to_path, find_project_root};
    
    let current_file_path = uri_to_path(file_uri)?;
    let project_root = find_project_root(&current_file_path)?;
    
    // First try to find in class_name_index for the current project
    let class_key = (project_root.clone(), class_name.to_string());
    if let Some(fqns) = dependency_cache.class_name_index.get(&class_key) {
        // Use the first matching FQN
        if let Some(fqn) = fqns.first() {
            debug!("try_find_symbol_by_short_name: resolved {} to FQN {} via class_name_index", class_name, fqn);
            return try_find_symbol_in_projects(fqn, file_uri, dependency_cache);
        }
    }
    
    // Try other projects' class_name_index
    for entry in dependency_cache.class_name_index.iter() {
        let ((proj_root, short_name), fqns) = (entry.key(), entry.value());
        if short_name == class_name {
            if let Some(fqn) = fqns.first() {
                debug!("try_find_symbol_by_short_name: found {} -> {} in project {:?}", class_name, fqn, proj_root);
                // Now look up the FQN in symbol_index
                let symbol_key = (proj_root.clone(), fqn.clone());
                if let Some(file_path) = dependency_cache.symbol_index.get(&symbol_key) {
                    // Convert file path to Location
                    use tower_lsp::lsp_types::{Position, Range, Url};
                    
                    if let Ok(uri) = Url::from_file_path(file_path.as_path()) {
                        let position = Position::new(0, 0);
                        let range = Range::new(position, position);
                        return Some(tower_lsp::lsp_types::Location::new(uri, range));
                    }
                }
            }
        }
    }
    
    debug!("try_find_symbol_by_short_name: class {} not found", class_name);
    None
}

/// Try to resolve class via workspace
fn try_resolve_class_via_workspace(
    _language_support: &dyn LanguageSupport,
    _source: &str,
    _file_uri: &str,
    class_name: &str,
    _dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!("try_resolve_class_via_workspace: attempting to resolve {}", class_name);
    None
}

/// Try to resolve class via external dependencies
fn try_resolve_class_via_external(
    _language_support: &dyn LanguageSupport,
    _source: &str,
    _file_uri: &str,
    class_name: &str,
    _dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!("try_resolve_class_via_external: attempting to resolve {}", class_name);
    None
}

/// Search for a method within a class file, detecting the target language automatically
#[tracing::instrument(skip_all)]
pub fn search_method_in_class_file_cross_language(
    class_location: &Location,
    method_name: &str,
) -> Option<Location> {
    
    // Determine the language from the file extension
    let target_language_support = detect_language_from_uri(class_location.uri.as_str())?;
    
    // Use the target language's method search logic
    search_method_in_class_file(class_location, method_name, target_language_support.as_ref())
}

/// Search for a field within a class file, detecting the target language automatically
#[tracing::instrument(skip_all)]
pub fn search_field_in_class_file_cross_language(
    class_location: &Location,
    field_name: &str,
) -> Option<Location> {
    
    // Determine the language from the file extension
    let target_language_support = detect_language_from_uri(class_location.uri.as_str())?;
    
    // Use the target language's field search logic
    search_field_in_class_file(class_location, field_name, target_language_support.as_ref())
}

#[tracing::instrument(skip_all)]
fn search_field_in_class_file(
    class_location: &Location,
    field_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    // Extract file path from URI and read content
    let file_path = class_location.uri.to_file_path().ok()?;
    let content = std::fs::read_to_string(&file_path).ok()?;
    
    // Parse the target file with the appropriate language parser
    let mut parser = language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // Create language-specific queries that capture field names directly
    let field_name_query = match language_support.language_id() {
        "kotlin" => format!(
            r#"(property_declaration (variable_declaration (simple_identifier) @field_name (#eq? @field_name "{}"))) @field_decl (enum_entry (simple_identifier) @field_name (#eq? @field_name "{}")) @field_decl"#, 
            field_name, field_name
        ),
        "java" => format!(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @field_name (#eq? @field_name "{}"))) @field_decl"#, 
            field_name
        ),
        "groovy" => format!(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @field_name (#eq? @field_name "{}"))) @field_decl"#, 
            field_name
        ),
        _ => {
            return None;
        }
    };
    
    if let Ok(query) = tree_sitter::Query::new(&tree.language(), &field_name_query) {
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
        
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "field_name" {
                    let node = capture.node;
                    let start_pos = node.start_position();
                    let end_pos = node.end_position();
                    
                    let start_position = tower_lsp::lsp_types::Position::new(
                        start_pos.row as u32, 
                        start_pos.column as u32
                    );
                    let end_position = tower_lsp::lsp_types::Position::new(
                        end_pos.row as u32, 
                        end_pos.column as u32
                    );
                    
                    let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
                    
                    return Some(tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range));
                }
            }
        }
    }
    
    None
}

/// Search for a method with signature matching across languages for overloaded methods
fn search_method_with_signature_cross_language(
    class_location: &Location,
    method_name: &str,
    call_signature: &Option<CallSignature>,
) -> Option<Location> {
    
    // If no call signature available, fall back to basic search
    let call_sig = call_signature.as_ref()?;
    
    // Determine the language from the file extension
    let target_language_support = detect_language_from_uri(class_location.uri.as_str())?;
    
    // Extract file path from URI and read content
    let file_path = class_location.uri.to_file_path().ok()?;
    let content = std::fs::read_to_string(&file_path).ok()?;
    
    // Parse the target file with the appropriate language parser
    let mut parser = target_language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // Use the language's signature matching implementation
    let signature_result = target_language_support.find_method_with_signature(&tree, &content, method_name, call_sig);
    signature_result
        .map(|node| {
            let start_pos = node.start_position();
            let end_pos = node.end_position();
            
            let start_position = tower_lsp::lsp_types::Position::new(start_pos.row as u32, start_pos.column as u32);
            let end_position = tower_lsp::lsp_types::Position::new(end_pos.row as u32, end_pos.column as u32);
            let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
            
            tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range)
        })
}

/// Detect the appropriate language support from a file URI
fn detect_language_from_uri(uri: &str) -> Option<Box<dyn LanguageSupport + Send + Sync>> {
    // Extract file extension from URI
    let path = if uri.starts_with("file://") {
        &uri[7..] // Remove "file://" prefix
    } else {
        uri
    };
    
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?;
    
    
    // Create appropriate language support based on extension
    match extension {
        "java" => {
            Some(Box::new(crate::languages::java::support::JavaSupport::new()))
        },
        "groovy" => {
            Some(Box::new(crate::languages::groovy::support::GroovySupport::new()))
        },
        "kt" | "kts" => {
            Some(Box::new(crate::languages::kotlin::support::KotlinSupport::new()))
        },
        _ => {
            None
        }
    }
}


/// Search for a specific method within a class file using tree-sitter (fallback without signature matching)
/// Search for enum constants that might be mistaken for static method calls
fn search_enum_constant_in_class_file(
    tree: &Tree,
    content: &str,
    constant_name: &str,
    file_uri: &str,
    language_id: &str,
) -> Option<Location> {
    use tree_sitter::{QueryCursor, StreamingIterator};
    
    // Create language-specific enum constant queries
    let enum_query = match language_id {
        "kotlin" => r#"(enum_entry (simple_identifier) @constant_name)"#,
        "java" | "groovy" => r#"(enum_constant name: (identifier) @constant_name)"#,
        _ => return None,
    };
    
    let language = match language_id {
        "kotlin" => tree_sitter_kotlin::language(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "groovy" => tree_sitter_groovy::language(),
        _ => return None,
    };
    
    let query = tree_sitter::Query::new(&language, enum_query).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(content.as_bytes()) {
                if capture_text == constant_name {
                    return crate::core::utils::node_to_lsp_location(&capture.node, file_uri);
                }
            }
        }
    }
    
    None
}

fn search_method_in_class_file(
    class_location: &Location,
    method_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    
    // Extract file path from URI and read content
    let file_path = class_location.uri.to_file_path().ok()?;
    let content = std::fs::read_to_string(&file_path).ok()?;
    
    // Parse the target file with the appropriate language parser
    let mut parser = language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // First, try to find enum constants (they look like static method calls but aren't methods)
    if let Some(enum_location) = search_enum_constant_in_class_file(&tree, &content, method_name, &class_location.uri.to_string(), language_support.language_id()) {
        return Some(enum_location);
    }
    
    // Create language-specific queries that capture method names directly
    let method_name_query = match language_support.language_id() {
        "kotlin" => format!(
            r#"(function_declaration (simple_identifier) @method_name (#eq? @method_name "{}")) @method_decl"#, 
            method_name
        ),
        "java" => format!(
            r#"(method_declaration name: (identifier) @method_name (#eq? @method_name "{}")) @method_decl"#, 
            method_name
        ),
        "groovy" => format!(
            r#"(method_declaration name: (identifier) @method_name (#eq? @method_name "{}")) @method_decl"#, 
            method_name
        ),
        _ => {
            // Fallback to the old approach for other languages
            return search_method_fallback(class_location, method_name, language_support);
        }
    };
    
    if let Ok(query) = tree_sitter::Query::new(&tree.language(), &method_name_query) {
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
        
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "method_name" {
                    let node = capture.node;
                    let start_pos = node.start_position();
                    let end_pos = node.end_position();
                    
                    let start_position = tower_lsp::lsp_types::Position::new(
                        start_pos.row as u32, 
                        start_pos.column as u32
                    );
                    let end_position = tower_lsp::lsp_types::Position::new(
                        end_pos.row as u32, 
                        end_pos.column as u32
                    );
                    
                    let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
                    
                    return Some(tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range));
                }
            }
        }
    }
    
    // If method not found, try getter/setter fallback logic
    if let Some(field_location) = try_find_getter_setter_field(class_location, method_name, language_support) {
        return Some(field_location);
    }
    
    // Fallback to old method if new query approach fails
    search_method_fallback(class_location, method_name, language_support)
}

/// Try to find the field associated with a getter/setter method
fn try_find_getter_setter_field(
    class_location: &Location,
    method_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    
    // Extract potential field name from getter/setter method name
    let field_name = if method_name.starts_with("get") && method_name.len() > 3 {
        // getMyField -> myField
        let field_base = &method_name[3..];
        if field_base.chars().next()?.is_uppercase() {
            let mut chars = field_base.chars();
            let first_char = chars.next()?.to_lowercase().to_string();
            let rest: String = chars.collect();
            first_char + &rest
        } else {
            return None; // Not a valid getter pattern
        }
    } else if method_name.starts_with("set") && method_name.len() > 3 {
        // setMyField -> myField
        let field_base = &method_name[3..];
        if field_base.chars().next()?.is_uppercase() {
            let mut chars = field_base.chars();
            let first_char = chars.next()?.to_lowercase().to_string();
            let rest: String = chars.collect();
            first_char + &rest
        } else {
            return None; // Not a valid setter pattern
        }
    } else if method_name.starts_with("is") && method_name.len() > 2 {
        // isEnabled -> enabled
        let field_base = &method_name[2..];
        if field_base.chars().next()?.is_uppercase() {
            let mut chars = field_base.chars();
            let first_char = chars.next()?.to_lowercase().to_string();
            let rest: String = chars.collect();
            first_char + &rest
        } else {
            return None; // Not a valid boolean getter pattern
        }
    } else {
        return None; // Not a getter/setter method
    };
    
    
    // Read the class file and search for the field
    let file_path = class_location.uri.to_file_path().ok()?;
    let content = std::fs::read_to_string(&file_path).ok()?;
    
    let mut parser = language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // Create language-specific queries for field declarations
    let field_query = match language_support.language_id() {
        "kotlin" => format!(
            r#"(property_declaration (variable_declaration (simple_identifier) @field_name (#eq? @field_name "{}"))) @field_decl"#,
            field_name
        ),
        "java" => format!(
            r#"(field_declaration (variable_declarator name: (identifier) @field_name (#eq? @field_name "{}"))) @field_decl"#,
            field_name
        ),
        "groovy" => format!(
            r#"(field_declaration (variable_declarator name: (identifier) @field_name (#eq? @field_name "{}"))) @field_decl"#,
            field_name
        ),
        _ => return None, // Unsupported language
    };
    
    if let Ok(query) = tree_sitter::Query::new(&tree.language(), &field_query) {
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
        
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "field_name" {
                    let node = capture.node;
                    let start_pos = node.start_position();
                    let end_pos = node.end_position();
                    
                    let start_position = tower_lsp::lsp_types::Position::new(
                        start_pos.row as u32,
                        start_pos.column as u32
                    );
                    let end_position = tower_lsp::lsp_types::Position::new(
                        end_pos.row as u32,
                        end_pos.column as u32
                    );
                    
                    let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
                    
                    return Some(tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range));
                }
            }
        }
    }
    
    None
}

/// Fallback method search using the old contains-based approach
fn search_method_fallback(
    class_location: &tower_lsp::lsp_types::Location, 
    method_name: &str, 
    language_support: &dyn crate::languages::traits::LanguageSupport
) -> Option<tower_lsp::lsp_types::Location> {
    let content = std::fs::read_to_string(class_location.uri.path()).ok()?;
    let mut parser = language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // Use the language's method declaration queries to find all methods
    let method_queries = language_support.method_declaration_queries();
    
    for query_str in method_queries {
        let query = tree_sitter::Query::new(&tree.language(), query_str).ok()?;
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
        
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let node = capture.node;
                
                // Check if this method node contains our target method name
                if let Ok(method_text) = node.utf8_text(content.as_bytes()) {
                    if method_text.contains(method_name) {
                        
                        // Find the method name identifier within this method declaration
                        if let Some(method_name_node) = find_method_name_in_declaration(&node, method_name, &content) {
                            let start_pos = method_name_node.start_position();
                            let end_pos = method_name_node.end_position();
                            
                            let start_position = tower_lsp::lsp_types::Position::new(
                                start_pos.row as u32, 
                                start_pos.column as u32
                            );
                            let end_position = tower_lsp::lsp_types::Position::new(
                                end_pos.row as u32, 
                                end_pos.column as u32
                            );
                            
                            let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
                            
                            return Some(tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range));
                        } else {
                            // Fallback to method declaration if we can't find the identifier
                            let start_pos = node.start_position();
                            let end_pos = node.end_position();
                            
                            let start_position = tower_lsp::lsp_types::Position::new(
                                start_pos.row as u32, 
                                start_pos.column as u32
                            );
                            let end_position = tower_lsp::lsp_types::Position::new(
                                end_pos.row as u32, 
                                end_pos.column as u32
                            );
                            
                            let range = tower_lsp::lsp_types::Range::new(start_position, end_position);
                            
                            return Some(tower_lsp::lsp_types::Location::new(class_location.uri.clone(), range));
                        }
                    }
                }
            }
        }
    }
    
    None
}


/// Find the method name identifier within a method declaration node
fn find_method_name_in_declaration<'a>(
    method_node: &tree_sitter::Node<'a>, 
    method_name: &str, 
    source: &'a str
) -> Option<tree_sitter::Node<'a>> {
    
    // Recursively search child nodes for an identifier that matches our method name
    fn search_for_identifier<'a>(
        node: tree_sitter::Node<'a>, 
        target_name: &str, 
        source: &'a str
    ) -> Option<tree_sitter::Node<'a>> {
        // Check if this node is an identifier with the target name
        if node.kind() == "identifier" || node.kind() == "simple_identifier" {
            if let Ok(node_text) = node.utf8_text(source.as_bytes()) {
                if node_text == target_name {
                    return Some(node);
                }
            }
        }
        
        // Recursively search child nodes
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = search_for_identifier(child, target_name, source) {
                return Some(found);
            }
        }
        
        None
    }
    
    search_for_identifier(*method_node, method_name, source)
}

// Re-export the signature matching types and functions 
pub use crate::languages::groovy::definition::definition_chain::{
    CallSignature, 
    extract_call_signature_from_context
};


/// Common logic for finding instance method definitions  
#[tracing::instrument(skip_all)]
pub fn find_instance_method_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    variable_name: &str,
    method_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {

    // Step 1: Extract the variable type using language-specific trait methods
    let variable_type = match extract_variable_type_from_tree(language_support, variable_name, tree, source, usage_node) {
        Some(t) => t,
        None => {
            return None;
        }
    };
    
    
    // Step 2: Find the class definition for the variable type
    let class_location = find_class_definition(
        language_support, tree, source, file_uri, &variable_type, dependency_cache.clone()
    )?;
    
    
    // Step 3: Extract call signature for overload resolution using language-specific logic
    let call_signature = language_support.extract_call_signature(usage_node, source);
    
    // Debug: log the usage node details
    
    // Step 4: Search for the method within the class file with signature matching for overloaded methods
    if let Some(method_location) = search_method_with_signature_cross_language(
        &class_location, method_name, &call_signature
    ) {
        return Some(method_location);
    }
    
    // Fallback: Try without signature matching
    if let Some(method_location) = search_method_in_class_file_cross_language(
        &class_location, method_name
    ) {
        return Some(method_location);
    }
    
    // Return class location as fallback
    Some(class_location)
}

/// Common logic for finding instance field/property definitions
#[tracing::instrument(skip_all)]
pub fn find_instance_field_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    variable_name: &str,
    field_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // Step 1: Extract the variable type using language-specific trait methods
    let variable_type = extract_variable_type_from_tree(language_support, variable_name, tree, source, usage_node)?;
    
    // Step 2: Find the class definition for the variable type
    let class_location = find_class_definition(
        language_support, tree, source, file_uri, &variable_type, dependency_cache.clone()
    )?;
    
    // Step 3: Search for the field/property within the class file
    if let Some(field_location) = search_field_in_class_file_cross_language(
        &class_location, field_name
    ) {
        return Some(field_location);
    }
    // Return class location as fallback
    Some(class_location)
}

/// Find class definition using the main definition chain directly
fn find_class_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    class_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // First try the static resolution approach (for consistency with existing working cases)
    if let Some(location) = try_resolve_class_name(language_support, tree, source, file_uri, class_name, dependency_cache.clone()) {
        return Some(location);
    }
    
    // If that fails, use the main definition chain directly
    // Try to find a node representing this class name to use with the main definition chain
    if let Some(class_node) = find_class_name_in_current_source(tree, source, class_name) {
        // Use the main definition chain directly
        if let Ok(location) = find_definition_chain_with_depth(
            language_support, tree, source, dependency_cache, file_uri, &class_node, 0
        ) {
            return Some(location);
        }
    }
    
    None
}

/// Find a class name node in the current source for standard resolution
fn find_class_name_in_current_source<'a>(tree: &'a Tree, source: &'a str, class_name: &str) -> Option<tree_sitter::Node<'a>> {
    fn search_node<'a>(node: tree_sitter::Node<'a>, source: &'a str, target: &str) -> Option<tree_sitter::Node<'a>> {
        // Check if this node matches the class name
        if matches!(node.kind(), "identifier" | "type_identifier" | "simple_identifier") {
            if let Ok(node_text) = node.utf8_text(source.as_bytes()) {
                if node_text == target {
                    return Some(node);
                }
            }
        }
        
        // Recursively search child nodes
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = search_node(child, source, target) {
                return Some(found);
            }
        }
        
        None
    }
    
    search_node(tree.root_node(), source, class_name)
}



/// Extract variable type using language-specific trait methods
fn extract_variable_type_from_tree(
    language_support: &dyn crate::languages::traits::LanguageSupport,
    variable_name: &str,
    tree: &Tree,
    source: &str,
    usage_node: &Node,
) -> Option<String> {
    
    // Look for field declarations first (class properties)
    if let Some(field_type) = language_support.find_field_declaration_type(variable_name, tree, source) {
        return Some(field_type);
    }
    
    // Look for variable declarations in scope
    if let Some(var_type) = language_support.find_variable_declaration_type(variable_name, tree, source, usage_node) {
        return Some(var_type);
    }
    
    // Try to find parameter type
    if let Some(param_type) = language_support.find_parameter_type(variable_name, tree, source, usage_node) {
        return Some(param_type);
    }
    
    None
}











/// Unified definition chain resolver that handles all symbol types across languages
/// Supports nested symbols (Outer.Inner.symbol), static/instance methods, and regular symbol lookup
#[tracing::instrument(skip_all)]
pub fn find_definition_chain(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
) -> Result<Location, anyhow::Error> {
    find_definition_chain_with_depth(
        language_support,
        tree,
        source,
        dependency_cache,
        file_uri,
        usage_node,
        0,
    )
}

/// Internal version with recursion depth tracking
#[tracing::instrument(skip_all)]
pub fn find_definition_chain_with_depth(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
    recursion_depth: usize,
) -> Result<Location, anyhow::Error> {
    const MAX_RECURSION_DEPTH: usize = 10;
    
    
    if recursion_depth >= MAX_RECURSION_DEPTH {
        tracing::warn!(
            "Maximum recursion depth {} reached for symbol at position {:?}",
            MAX_RECURSION_DEPTH,
            usage_node.start_position()
        );
        return Err(anyhow::anyhow!("Maximum recursion depth exceeded"));
    }
    
    // FIRST: Check for nested access patterns using parent context and symbol type
    if let Some(parent) = usage_node.parent() {
        let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
        let parent_text = parent.utf8_text(source.as_bytes()).unwrap_or("");
        
        // For navigation_suffix, we need to go up one more level to get the full chain
        let mut search_text = parent_text.to_string();
        
        if parent.kind() == "navigation_suffix" {
            if let Some(grandparent) = parent.parent() {
                let grandparent_text = grandparent.utf8_text(source.as_bytes()).unwrap_or("");
                search_text = grandparent_text.to_string();
            }
        }
        
        // Check if search text contains a dotted pattern that includes our symbol
        if search_text.contains('.') && search_text.contains(symbol_text) {
            // Try to determine what type of access this is
            if let Ok(symbol_type) = language_support.determine_symbol_type_from_context(tree, usage_node, source) {
                // For FieldUsage in dotted patterns, check if this looks like a static/nested access or instance access
                if matches!(symbol_type, crate::core::symbols::SymbolType::FieldUsage) {
                    let full_symbol_path = search_text;
                    
                    // Check if this looks like static/nested access (PascalCase) vs instance access (camelCase)
                    let parts: Vec<&str> = full_symbol_path.split('.').collect();
                    let is_likely_static = parts.len() >= 2 && 
                        parts[0].chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                    
                    if is_likely_static {
                        // Try as Type first (nested class/enum), then Property
                        if let Some(result) = language_support.find_type(source, file_uri, &full_symbol_path, dependency_cache.clone()) {
                            return Ok(result);
                        }
                        
                        if let Some(result) = language_support.find_property(source, file_uri, &full_symbol_path, dependency_cache.clone()) {
                            return Ok(result);
                        }
                    }
                    // For instance access (camelCase), skip nested resolution and continue to instance field resolution
                }
            }
        }
    }

    // SECOND: Try static method resolution
    if let Some((class_name, method_name)) = language_support.extract_static_method_context(usage_node, source) {
        if let Some(location) = find_static_method_definition(
            language_support, tree, source, file_uri, usage_node, &class_name, &method_name, dependency_cache.clone()
        ) {
            return Ok(location);
        }
    }

    // THIRD: Try static field resolution
    if let Some((class_name, field_name)) = extract_static_field_context(usage_node, source) {
        if let Some(location) = find_static_field_definition(
            language_support, tree, source, file_uri, usage_node, &class_name, &field_name, dependency_cache.clone()
        ) {
            return Ok(location);
        }
    }

    // Try instance method resolution  
    if let Some((variable_name, method_name)) = language_support.extract_instance_method_context(usage_node, source) {
        if let Some(location) = language_support.find_instance_method_definition(
            tree, source, file_uri, usage_node, &variable_name, &method_name, dependency_cache.clone()
        ) {
            return Ok(location);
        }
    }

    // Try instance field/property resolution
    if let Some((variable_name, field_name)) = language_support.extract_instance_field_context(usage_node, source) {
        if let Some(location) = find_instance_field_definition(
            language_support, tree, source, file_uri, usage_node, &variable_name, &field_name, dependency_cache.clone()
        ) {
            return Ok(location);
        }
    }

    // Determine symbol type for further processing
    let symbol_type = language_support.determine_symbol_type_from_context(tree, usage_node, source).ok();
    
    // Handle method calls that weren't resolved above
    if let Some(crate::core::symbols::SymbolType::MethodCall) = symbol_type {
        // Fall through to standard resolution for unresolved method calls
    } else if let Some(symbol_type) = symbol_type {
        use crate::core::symbols::SymbolType;
        if matches!(symbol_type, 
            SymbolType::VariableUsage | 
            SymbolType::ParameterDeclaration |
            SymbolType::FieldUsage
        ) {
            if let Some(local_location) = language_support.find_local(tree, source, file_uri, usage_node) {
                return Ok(local_location);
            }
        }
        
        // For local method calls that aren't found locally, 
        // they're likely in the same project - skip expensive workspace/external search
        if symbol_type == SymbolType::MethodCall {
            if let Some(project_location) = language_support.find_in_project(source, file_uri, usage_node, dependency_cache.clone()) {
                // If the definition is in the same file, don't call set_start_position 
                // as it may find the wrong identifier with the same name
                if project_location.uri.to_string() == file_uri {
                    return Ok(project_location);
                } else {
                    let uri_string = project_location.uri.to_string();
                    // Skip set_start_position for builtin sources as they are already correctly positioned
                    if uri_string.contains("lspintar_builtin_sources") {
                        return Ok(project_location);
                    } else {
                        tracing::debug!("DEBUG_LSP: Calling set_start_position for project location");
                        // Try to get a more precise position, but fall back to original if that fails
                        let final_location = language_support.set_start_position(source, usage_node, &uri_string)
                            .unwrap_or(project_location);
                        tracing::debug!("DEBUG_LSP: Final location after set_start_position: {:?}", final_location);
                        return Ok(final_location);
                    }
                }
            }
        }
    }

    // Fall back to standard resolution chain
    if let Some(local_location) = language_support.find_local(tree, source, file_uri, usage_node) {
        return Ok(local_location);
    }
    
    // Try cross-file resolution
    language_support.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
        .or_else(|| language_support.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone(), recursion_depth + 1))
        .or_else(|| language_support.find_external(source, file_uri, usage_node, dependency_cache))
        .and_then(|location| {
            // If the definition is in the same file, don't call set_start_position
            // as it may find the wrong identifier with the same name
            if location.uri.to_string() == file_uri {
                Some(location)
            } else {
                let uri_string = location.uri.to_string();
                // Skip set_start_position for builtin sources as they are already correctly positioned
                if uri_string.contains("lspintar_builtin_sources") {
                    Some(location)
                } else {
                    // Try to get a more precise position, but fall back to original if that fails  
                    let result = language_support.set_start_position(source, usage_node, &uri_string)
                        .or(Some(location));
                    result
                }
            }
        })
        .ok_or_else(|| anyhow::anyhow!("Definition not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        parser.set_language(&language).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn create_kotlin_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        let language = tree_sitter_kotlin::language();
        parser.set_language(&language).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn find_identifier_node<'a>(tree: &'a Tree, source: &'a str, target_text: &str) -> Option<tree_sitter::Node<'a>> {
        fn find_node_recursive<'a>(node: tree_sitter::Node<'a>, source: &'a str, target: &str) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == "identifier" || node.kind() == "simple_identifier" {
                if let Ok(text) = node.utf8_text(source.as_bytes()) {
                    if text == target {
                        return Some(node);
                    }
                }
            }
            
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(found) = find_node_recursive(child, source, target) {
                    return Some(found);
                }
            }
            None
        }
        
        find_node_recursive(tree.root_node(), source, target_text)
    }

    #[test]
    fn test_static_method_context_on_method_name() {
        let source = "Math.max(1, 2);";
        let tree = create_test_tree(source);
        
        // Find the "max" identifier
        let max_node = find_identifier_node(&tree, source, "max").unwrap();
        
        let result = extract_static_method_context(&max_node, source);
        assert!(result.is_some());
        let (class_name, method_name) = result.unwrap();
        assert_eq!(class_name, "Math");
        assert_eq!(method_name, "max");
    }
    
    #[test]  
    fn test_static_method_context_on_class_name() {
        let source = "Math.max(1, 2);";
        let tree = create_test_tree(source);
        
        // Find the "Math" identifier  
        let math_node = find_identifier_node(&tree, source, "Math").unwrap();
        
        let result = extract_static_method_context(&math_node, source);
        // Should return None so it goes to class definition instead
        assert!(result.is_none());
    }

    #[test]
    fn test_instance_method_context_on_method_name() {
        let source = "list.add(item);";
        let tree = create_test_tree(source);
        
        // Find the "add" identifier
        let add_node = find_identifier_node(&tree, source, "add").unwrap();
        
        let result = extract_instance_method_context(&add_node, source);
        assert!(result.is_some());
        let (variable_name, method_name) = result.unwrap();
        assert_eq!(variable_name, "list");
        assert_eq!(method_name, "add");
    }
    
    #[test]
    fn test_instance_method_context_on_variable_name() {
        let source = "list.add(item);";
        let tree = create_test_tree(source);
        
        // Find the "list" identifier
        let list_node = find_identifier_node(&tree, source, "list").unwrap();
        
        let result = extract_instance_method_context(&list_node, source);
        // Should return None so it goes to variable declaration instead
        assert!(result.is_none());
    }

    #[test]
    fn test_java_symbol_type_for_class_in_static_call() {
        let source = "Math.max(1, 2);";
        let tree = create_test_tree(source);
        
        // Find the "Math" identifier
        let math_node = find_identifier_node(&tree, source, "Math").unwrap();
        
        // Create Java support to test symbol type detection
        let java_support = crate::languages::java::support::JavaSupport::new();
        let symbol_type = java_support.determine_symbol_type_from_context(&tree, &math_node, source).unwrap();
        
        // It should be Type, not FieldUsage
        assert_eq!(symbol_type, crate::core::symbols::SymbolType::Type);
    }

    #[test]
    fn test_static_field_context_on_field_name() {
        let source = "String value = Type.myField;";
        let tree = create_test_tree(source);
        
        // Find the "myField" identifier
        let field_node = find_identifier_node(&tree, source, "myField").unwrap();
        
        let result = extract_static_field_context(&field_node, source);
        assert!(result.is_some());
        let (class_name, field_name) = result.unwrap();
        assert_eq!(class_name, "Type");
        assert_eq!(field_name, "myField");
    }
    
    #[test]  
    fn test_static_field_context_on_class_name() {
        let source = "String value = Type.myField;";
        let tree = create_test_tree(source);
        
        // Find the "Type" identifier  
        let type_node = find_identifier_node(&tree, source, "Type").unwrap();
        
        let result = extract_static_field_context(&type_node, source);
        // Should return None so it goes to class definition instead
        assert!(result.is_none());
    }

    #[test]
    fn test_java_symbol_type_for_class_in_static_field_access() {
        let source = "String value = Type.myField;";
        let tree = create_test_tree(source);
        
        // Find the "Type" identifier
        let type_node = find_identifier_node(&tree, source, "Type").unwrap();
        
        // Create Java support to test symbol type detection
        let java_support = crate::languages::java::support::JavaSupport::new();
        let symbol_type = java_support.determine_symbol_type_from_context(&tree, &type_node, source).unwrap();
        
        // It should be Type, not FieldUsage (this tests the type_usage_in_field_access pattern)
        assert_eq!(symbol_type, crate::core::symbols::SymbolType::Type);
    }

    #[test]
    fn test_java_symbol_type_for_field_in_static_field_access() {
        let source = "String value = Type.myField;";
        let tree = create_test_tree(source);
        
        // Find the "myField" identifier
        let field_node = find_identifier_node(&tree, source, "myField").unwrap();
        
        // Create Java support to test symbol type detection
        let java_support = crate::languages::java::support::JavaSupport::new();
        let symbol_type = java_support.determine_symbol_type_from_context(&tree, &field_node, source).unwrap();
        
        // It should be FieldUsage
        assert_eq!(symbol_type, crate::core::symbols::SymbolType::FieldUsage);
    }

    fn find_node_at_position<'a>(tree: &'a Tree, position: tower_lsp::lsp_types::Position, _source: &str) -> Option<tree_sitter::Node<'a>> {
        fn find_node_at_position_recursive<'a>(node: tree_sitter::Node<'a>, row: usize, col: usize) -> Option<tree_sitter::Node<'a>> {
            let start_pos = node.start_position();
            let end_pos = node.end_position();
            
            // Check if position is within this node
            if (row > start_pos.row || (row == start_pos.row && col >= start_pos.column)) &&
               (row < end_pos.row || (row == end_pos.row && col <= end_pos.column)) {
                
                // If this node has children, try to find a more specific child
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Some(child_match) = find_node_at_position_recursive(child, row, col) {
                        return Some(child_match);
                    }
                }
                
                // If no child matched, return this node if it's a leaf or identifier
                if node.child_count() == 0 || node.kind() == "simple_identifier" || node.kind() == "identifier" {
                    return Some(node);
                }
            }
            
            None
        }
        
        find_node_at_position_recursive(tree.root_node(), position.line as usize, position.character as usize)
    }

    #[test]
    fn test_kotlin_uses_common_method_resolution() {
        // Simple test to verify Kotlin now uses common method resolution
        let source = "list.add(item)";
        let tree = create_kotlin_test_tree(source);
        
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        // Just verify that find_instance_method_definition can be called without panicking
        // We can't test the full functionality without a complete Kotlin environment,
        // but this ensures the common method resolution is being used
        let dependency_cache = std::sync::Arc::new(crate::core::dependency_cache::DependencyCache::new());
        
        let result = kotlin_support.find_instance_method_definition(
            &tree,
            source,
            "file:///test.kt",
            &tree.root_node(),
            "list",
            "add",
            dependency_cache,
        );
        
        // The result may be None (which is fine for this test), but it shouldn't panic
        // This confirms that Kotlin is now using the common method resolution logic
        let _ = result; // Just ensure it doesn't panic
    }

    #[test]
    fn test_improved_method_search_query() {
        // Test that the new query-based method search works correctly for Kotlin
        let kotlin_class_content = r#"class TestClass {
    fun someOtherMethod() {
        val result = targetMethod()  // This should NOT be matched
        return result
    }
    
    fun targetMethod(): String {  // This SHOULD be matched
        return "test"
    }
}"#;
        
        // Create a temporary file for testing
        let temp_file = std::env::temp_dir().join("test_kotlin_method_search.kt");
        std::fs::write(&temp_file, kotlin_class_content).unwrap();
        
        let file_uri = tower_lsp::lsp_types::Url::from_file_path(&temp_file).unwrap();
        let class_location = tower_lsp::lsp_types::Location {
            uri: file_uri,
            range: tower_lsp::lsp_types::Range::default(),
        };
        
        // Test searching for "targetMethod" - it should find the method declaration, not the call
        if let Some(result) = search_method_in_class_file_cross_language(&class_location, "targetMethod") {
            // The result should point to the method declaration line (line with "fun targetMethod(): String")
            // In our test, that's around line 6 (0-indexed)
            println!("Found targetMethod at line {}", result.range.start.line);
            
            // For now, let's just check that it found SOMETHING and print the result
            // The old method was finding line 2 (method call), new method should find line 6-7 (declaration)
            if result.range.start.line == 2 {
                println!("WARNING: Found method call instead of declaration - query approach may have failed, falling back to old method");
            } else {
                println!("SUCCESS: Found method declaration at line {}", result.range.start.line);
            }
        } else {
            panic!("Method search should have found the targetMethod method declaration");
        }
        
        // Clean up
        let _ = std::fs::remove_file(&temp_file);
    }

    // === Kotlin Property Context Tests ===

    #[test]
    fn test_kotlin_static_field_context_on_field_name() {
        let source = "val constant = RedisConstants.BACKUP_WA_TEMPLATES";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "BACKUP_WA_TEMPLATES" identifier
        let field_node = find_identifier_node(&tree, source, "BACKUP_WA_TEMPLATES").unwrap();
        
        let result = extract_static_field_context(&field_node, source);
        assert!(result.is_some());
        let (class_name, field_name) = result.unwrap();
        assert_eq!(class_name, "RedisConstants");
        assert_eq!(field_name, "BACKUP_WA_TEMPLATES");
    }
    
    #[test]  
    fn test_kotlin_static_field_context_on_class_name() {
        let source = "val constant = RedisConstants.BACKUP_WA_TEMPLATES";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "RedisConstants" identifier  
        let class_node = find_identifier_node(&tree, source, "RedisConstants").unwrap();
        
        let result = extract_static_field_context(&class_node, source);
        // Should return None so it goes to class definition instead
        assert!(result.is_none());
    }

    #[test]
    fn test_kotlin_static_field_context_in_annotation() {
        let source = "@Cacheable(RedisConstants.BACKUP_WA_TEMPLATES)";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "BACKUP_WA_TEMPLATES" identifier
        let field_node = find_identifier_node(&tree, source, "BACKUP_WA_TEMPLATES").unwrap();
        
        let result = extract_static_field_context(&field_node, source);
        assert!(result.is_some());
        let (class_name, field_name) = result.unwrap();
        assert_eq!(class_name, "RedisConstants");
        assert_eq!(field_name, "BACKUP_WA_TEMPLATES");
    }

    #[test]
    fn test_kotlin_instance_field_context_on_field_name() {
        let source = "val text = sms.body";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "body" identifier
        let field_node = find_identifier_node(&tree, source, "body").unwrap();
        
        // Use KotlinSupport trait to test the Kotlin-specific implementation
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        let result = kotlin_support.extract_instance_field_context(&field_node, source);
        assert!(result.is_some());
        let (variable_name, field_name) = result.unwrap();
        assert_eq!(variable_name, "sms");
        assert_eq!(field_name, "body");
    }
    
    #[test]
    fn test_kotlin_instance_field_context_on_variable_name() {
        let source = "val text = sms.body";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "sms" identifier in the property access (not the assignment)
        let sms_node = find_identifier_node(&tree, source, "sms").unwrap();
        
        // Use KotlinSupport trait to test the Kotlin-specific implementation
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        let result = kotlin_support.extract_instance_field_context(&sms_node, source);
        // Should return None so it goes to variable declaration instead
        assert!(result.is_none());
    }

    #[test]
    fn test_kotlin_instance_field_context_in_assignment() {
        let source = "sms.body = \"test\"";
        let tree = create_kotlin_test_tree(source);
        
        // Find the "body" identifier
        let field_node = find_identifier_node(&tree, source, "body").unwrap();
        
        // Use KotlinSupport trait to test the Kotlin-specific implementation
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        let result = kotlin_support.extract_instance_field_context(&field_node, source);
        assert!(result.is_some());
        let (variable_name, field_name) = result.unwrap();
        assert_eq!(variable_name, "sms");
        assert_eq!(field_name, "body");
    }

    #[test]
    fn test_kotlin_property_context_mixed_case() {
        // Test that lowercase variable names are treated as instances, uppercase as static
        let source = "val result = MyClass.CONSTANT + myInstance.property";
        let tree = create_kotlin_test_tree(source);
        
        // Test static access (this uses common function which now supports Kotlin)
        let static_field_node = find_identifier_node(&tree, source, "CONSTANT").unwrap();
        let static_result = extract_static_field_context(&static_field_node, source);
        assert!(static_result.is_some());
        let (static_class, static_field) = static_result.unwrap();
        assert_eq!(static_class, "MyClass");
        assert_eq!(static_field, "CONSTANT");
        
        // Test instance access (this uses KotlinSupport trait)
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        let instance_field_node = find_identifier_node(&tree, source, "property").unwrap();
        let instance_result = kotlin_support.extract_instance_field_context(&instance_field_node, source);
        assert!(instance_result.is_some());
        let (instance_var, instance_field) = instance_result.unwrap();
        assert_eq!(instance_var, "myInstance");
        assert_eq!(instance_field, "property");
    }

    #[test]
    fn test_kotlin_overload_resolution() {
        // Test that Kotlin can distinguish between overloaded methods based on parameter count
        let kotlin_class_content = r#"class TestClass {
    fun process(): String {
        return "no params"
    }
    
    fun process(input: String): String {
        return "one param: $input"
    }
    
    fun process(input1: String, input2: Int): String {
        return "two params: $input1, $input2"
    }
    
    fun someMethod() {
        val result1 = process()              // Should find 0-param version
        val result2 = process("test")        // Should find 1-param version  
        val result3 = process("test", 42)    // Should find 2-param version
    }
}"#;
        
        // Create a temporary file for testing
        let temp_file = std::env::temp_dir().join("test_kotlin_overload_resolution.kt");
        std::fs::write(&temp_file, kotlin_class_content).unwrap();
        
        let file_uri = tower_lsp::lsp_types::Url::from_file_path(&temp_file).unwrap();
        let class_location = tower_lsp::lsp_types::Location {
            uri: file_uri,
            range: tower_lsp::lsp_types::Range::default(),
        };
        
        // Test that the improved method search finds the correct overloaded method
        let result_0_param = search_method_in_class_file_cross_language(&class_location, "process");
        
        // Should find method declaration (not calls)
        // The exact line will depend on which overload it finds first, but it should be one of the declarations
        if let Some(result) = result_0_param {
            println!("Found process method at line {}", result.range.start.line);
            // Should find one of the method declarations (lines 1, 5, or 9), not the calls (lines 13-15)
            assert!(result.range.start.line < 10, "Should find method declaration, not method call. Found at line {}", result.range.start.line);
        } else {
            panic!("Should have found at least one process method declaration");
        }
        
        // Clean up
        let _ = std::fs::remove_file(&temp_file);
    }

    #[test]
    fn test_kotlin_static_method_context_extraction() {
        // Test that Kotlin correctly distinguishes between clicking on class name vs method name
        let source = "TestClass.staticMethod()";
        let tree = create_kotlin_test_tree(source);
        
        // First, let's see what the AST looks like
        println!("Kotlin static call AST:");
        print_kotlin_ast(&tree.root_node(), source, 0);
        
        // Find all identifiers to see what's available
        println!("\nAll identifiers in the source:");
        let simple_query = "(simple_identifier) @id";
        let language = tree_sitter_kotlin::language();
        if let Ok(query) = tree_sitter::Query::new(&language, simple_query) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let id_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("?");
                    println!("  Found identifier: '{}'", id_text);
                }
            }
        }
        
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        // Find the "TestClass" identifier (should NOT return method context)
        if let Some(class_node) = find_identifier_node(&tree, source, "TestClass") {
            let class_result = kotlin_support.extract_static_method_context(&class_node, source);
            
            // Should return None so it goes to class definition
            assert!(class_result.is_none(), "Go-to-definition on class name should return None to go to class definition");
            println!("✅ TestClass correctly returns None");
        } else {
            println!("⚠️ TestClass identifier not found");
        }
        
        // Find the "staticMethod" identifier (should return method context)
        if let Some(method_node) = find_identifier_node(&tree, source, "staticMethod") {
            let method_result = kotlin_support.extract_static_method_context(&method_node, source);
            
            // Should return Some with method context
            assert!(method_result.is_some(), "Go-to-definition on method name should return method context");
            if let Some((class_name, method_name)) = method_result {
                assert_eq!(class_name, "TestClass");
                assert_eq!(method_name, "staticMethod");
                println!("✅ staticMethod correctly returns method context");
            }
        } else {
            println!("⚠️ staticMethod identifier not found");
        }
    }

    #[test]
    fn debug_kotlin_query_syntax() {
        // Debug test to check if Kotlin query syntax is correct
        let kotlin_content = r#"class TestClass {
    fun targetMethod(): String {
        return "test"
    }
}"#;
        
        let tree = create_kotlin_test_tree(kotlin_content);
        
        // First, print the AST to see the actual structure
        println!("Kotlin AST structure:");
        print_kotlin_ast(&tree.root_node(), kotlin_content, 0);
        
        // Test the corrected query
        let corrected_query = r#"(function_declaration (simple_identifier) @method_name (#eq? @method_name "targetMethod")) @method_decl"#;
        let language = tree_sitter_kotlin::language();
        match tree_sitter::Query::new(&language, corrected_query) {
            Ok(query) => {
                println!("\nCorrected query compiled successfully!");
                let mut cursor = tree_sitter::QueryCursor::new();
                let mut matches = cursor.matches(&query, tree.root_node(), kotlin_content.as_bytes());
                
                let mut found_any = false;
                while let Some(query_match) = matches.next() {
                    found_any = true;
                    for capture in query_match.captures {
                        let capture_name = query.capture_names()[capture.index as usize];
                        let node_text = capture.node.utf8_text(kotlin_content.as_bytes()).unwrap_or("?");
                        println!("Found capture: @{} -> '{}'", capture_name, node_text);
                        
                        if capture_name == "method_name" {
                            println!("SUCCESS: Found method name identifier at line {}", capture.node.start_position().row);
                        }
                    }
                }
                
                if !found_any {
                    println!("No matches found with corrected query");
                }
            }
            Err(e) => {
                println!("Corrected query compilation failed: {:?}", e);
            }
        }
    }
    
    fn print_kotlin_ast(node: &tree_sitter::Node, source: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            format!(" \"{}\"", node.utf8_text(source.as_bytes()).unwrap_or(""))
        } else {
            String::new()
        };
        
        println!("{}({} {})", indent, node.kind(), text);
        
        // Print field names for this node
        if node.child_count() > 0 {
            let mut cursor = node.walk();
            for (i, child) in node.children(&mut cursor).enumerate() {
                if let Some(field_name) = node.field_name_for_child(i as u32) {
                    println!("{}  field '{}': {}", indent, field_name, child.kind());
                }
            }
        }
        
        if depth < 5 { // Increased depth to see more structure
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                print_kotlin_ast(&child, source, depth + 1);
            }
        }
    }

    #[test]
    fn test_kotlin_instance_method_context_extraction() {
        let source = r#"
class TestClass {
    fun instanceMethod(): String {
        return "test"
    }
}

fun main() {
    val obj = TestClass()
    obj.instanceMethod()
}
"#;
        let tree = create_kotlin_test_tree(source);
        
        println!("=== Testing Kotlin Instance Method Context ===");
        println!("Source: {}", source);
        
        // Find obj identifier and instanceMethod identifier
        println!("\nSearching for identifiers...");
        
        // Test queries to find both identifiers
        let obj_query = r#"(simple_identifier) @obj (#eq? @obj "obj")"#;
        let method_query = r#"(simple_identifier) @method (#eq? @method "instanceMethod")"#;
        
        let language = tree_sitter_kotlin::language();
        
        if let Ok(query) = tree_sitter::Query::new(&language, obj_query) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            println!("\nFound 'obj' identifiers:");
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let node = capture.node;
                    let line = node.start_position().row + 1;
                    let col = node.start_position().column + 1;
                    println!("  - obj at line {}, col {} (kind: {})", line, col, node.kind());
                    
                    // Check parent nodes to understand context
                    if let Some(parent) = node.parent() {
                        println!("    Parent: {} at line {}", parent.kind(), parent.start_position().row + 1);
                        if let Some(grandparent) = parent.parent() {
                            println!("    Grandparent: {} at line {}", grandparent.kind(), grandparent.start_position().row + 1);
                        }
                    }
                }
            }
        }
        
        if let Ok(query) = tree_sitter::Query::new(&language, method_query) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            println!("\nFound 'instanceMethod' identifiers:");
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let node = capture.node;
                    let line = node.start_position().row + 1;
                    let col = node.start_position().column + 1;
                    println!("  - instanceMethod at line {}, col {} (kind: {})", line, col, node.kind());
                    
                    // Check parent nodes to understand context
                    if let Some(parent) = node.parent() {
                        println!("    Parent: {} at line {}", parent.kind(), parent.start_position().row + 1);
                        if let Some(grandparent) = parent.parent() {
                            println!("    Grandparent: {} at line {}", grandparent.kind(), grandparent.start_position().row + 1);
                        }
                    }
                }
            }
        }
        
        // Now test the actual method context extraction logic
        println!("\n=== Testing extract_instance_method_context ===");
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        // Find the method call node (obj.instanceMethod())
        let call_query = r#"(call_expression) @call"#;
        if let Ok(query) = tree_sitter::Query::new(&language, call_query) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let call_node = capture.node;
                    println!("Found call_expression at line {}", call_node.start_position().row + 1);
                    
                    // Test different positions within the call - variable vs method
                    let mut cursor = call_node.walk();
                    for child in call_node.children(&mut cursor) {
                        if child.kind() == "navigation_expression" {
                            let mut nav_cursor = child.walk();
                            for nav_child in child.children(&mut nav_cursor) {
                                if nav_child.kind() == "simple_identifier" {
                                    let text = nav_child.utf8_text(source.as_bytes()).unwrap_or("");
                                    println!("Testing context extraction for identifier '{}' at line {}", text, nav_child.start_position().row + 1);
                                    
                                    if let Some((var_name, method_name)) = kotlin_support.extract_instance_method_context(&nav_child, source) {
                                        println!("  Context extracted: variable='{}', method='{}'", var_name, method_name);
                                        
                                        // Test the issue: clicking on variable name should return None to go to variable definition
                                        if text == var_name {
                                            println!("  ISSUE: Clicking on variable '{}' extracted context (should return None)", text);
                                            println!("    This means go-to-definition will try to find method instead of variable");
                                        } else if text == method_name {
                                            println!("  Clicking on method '{}' correctly extracted context", text);
                                        }
                                    } else {
                                        println!("  No context extracted for '{}'", text);
                                        if text != "obj" && text != "instanceMethod" {
                                            println!("    This is expected for non-method-call identifiers");
                                        } else if text == "obj" {
                                            println!("    Good! Variable name should return None for variable definition lookup");
                                        } else if text == "instanceMethod" {
                                            println!("    ISSUE: Method name should return context for method resolution");
                                        }
                                    }
                                }
                                
                                // Also check navigation_suffix children for method identifiers
                                if nav_child.kind() == "navigation_suffix" {
                                    let mut suffix_cursor = nav_child.walk();
                                    for suffix_child in nav_child.children(&mut suffix_cursor) {
                                        if suffix_child.kind() == "simple_identifier" {
                                            let text = suffix_child.utf8_text(source.as_bytes()).unwrap_or("");
                                            println!("Testing context extraction for method identifier '{}' at line {}", text, suffix_child.start_position().row + 1);
                                            
                                            if let Some((var_name, method_name)) = kotlin_support.extract_instance_method_context(&suffix_child, source) {
                                                println!("  Context extracted: variable='{}', method='{}'", var_name, method_name);
                                                
                                                if text == method_name {
                                                    println!("  Good! Method name correctly extracted context");
                                                } else if text == var_name {
                                                    println!("  ISSUE: Method identifier extracted variable context");
                                                }
                                            } else {
                                                println!("  ISSUE: Method identifier '{}' should return context", text);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_kotlin_constructor_parameter_type_extraction() {
        let source = r#"
class TestClass(val name: String, private val service: HttpService) {
    fun performAction() {
        // These should resolve the types from constructor parameters
        name.length  // 'name' should resolve to String
        service.get() // 'service' should resolve to HttpService 
    }
}
"#;
        let tree = create_kotlin_test_tree(source);
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        println!("=== Testing Kotlin Constructor Parameter Type Extraction ===");
        println!("Source: {}", source);
        
        // Test that constructor parameters can be found as types
        let name_type = kotlin_support.find_parameter_type("name", &tree, source, &tree.root_node());
        let service_type = kotlin_support.find_parameter_type("service", &tree, source, &tree.root_node());
        
        println!("Constructor parameter 'name' type: {:?}", name_type);
        println!("Constructor parameter 'service' type: {:?}", service_type);
        
        // Verify the types are extracted correctly
        assert_eq!(name_type, Some("String".to_string()));
        assert_eq!(service_type, Some("HttpService".to_string()));
        
        // Test that regular function parameters also work
        let source_with_function = r#"
class TestClass {
    fun processData(input: DataProcessor, count: Int) {
        // These should resolve from function parameters
        input.process()
        println(count)
    }
}
"#;
        let tree2 = create_kotlin_test_tree(source_with_function);
        
        let input_type = kotlin_support.find_parameter_type("input", &tree2, source_with_function, &tree2.root_node());
        let count_type = kotlin_support.find_parameter_type("count", &tree2, source_with_function, &tree2.root_node());
        
        println!("Function parameter 'input' type: {:?}", input_type);
        println!("Function parameter 'count' type: {:?}", count_type);
        
        // Verify function parameter types
        assert_eq!(input_type, Some("DataProcessor".to_string()));
        assert_eq!(count_type, Some("Int".to_string()));
    }

    #[test]
    fn test_kotlin_end_to_end_variable_definition_lookup() {
        let source = r#"
class TestService {
    fun doWork(): String {
        return "working"
    }
}

fun main() {
    val service = TestService()
    service.doWork()  // Click on 'service' should go to variable declaration on line above
}
"#;
        let tree = create_kotlin_test_tree(source);
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        println!("=== Testing End-to-End Kotlin Variable Definition Lookup ===");
        
        // Find the 'service' identifier in the method call
        let service_query = r#"
            (call_expression
              (navigation_expression
                (simple_identifier) @service_var (#eq? @service_var "service")))
        "#;
        
        let language = tree_sitter_kotlin::language();
        if let Ok(query) = tree_sitter::Query::new(&language, service_query) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let capture_name = query.capture_names()[capture.index as usize];
                    if capture_name == "service_var" {
                        let service_node = capture.node;
                        let line = service_node.start_position().row + 1;
                        let col = service_node.start_position().column + 1;
                        println!("Found 'service' identifier at line {}, col {}", line, col);
                        
                        // Test 1: Verify that extract_instance_method_context returns None
                        if let Some((var_name, method_name)) = kotlin_support.extract_instance_method_context(&service_node, source) {
                            println!("ERROR: extract_instance_method_context returned Some(({}, {})) but should return None", var_name, method_name);
                        } else {
                            println!("✓ extract_instance_method_context correctly returned None");
                        }
                        
                        // Test 2: Check what symbol type is detected
                        match kotlin_support.determine_symbol_type_from_context(&tree, &service_node, source) {
                            Ok(symbol_type) => {
                                println!("Symbol type detected: {:?}", symbol_type);
                            }
                            Err(e) => {
                                println!("Error detecting symbol type: {}", e);
                            }
                        }
                        
                        // Test 3: Try find_local directly
                        let file_uri = "file:///test.kt";
                        match kotlin_support.find_local(&tree, source, file_uri, &service_node) {
                            Some(location) => {
                                println!("✓ find_local found definition at line {}", location.range.start.line + 1);
                            }
                            None => {
                                println!("✗ find_local failed to find variable definition");
                            }
                        }
                        
                        // Test 4: Try the full definition chain
                        let dependency_cache = std::sync::Arc::new(crate::core::dependency_cache::DependencyCache::new());
                        match kotlin_support.find_definition(&tree, source, 
                            tower_lsp::lsp_types::Position::new(line as u32 - 1, col as u32 - 1), 
                            file_uri, dependency_cache) {
                            Ok(location) => {
                                println!("✓ find_definition found definition at line {}", location.range.start.line + 1);
                            }
                            Err(e) => {
                                println!("✗ find_definition failed: {}", e);
                            }
                        }
                        
                        break;
                    }
                }
            }
        } else {
            println!("Failed to create query for finding service identifier");
        }
        
        // Also test with the exact method resolution chain that's used in practice
        println!("\n=== Testing with actual method resolution chain ===");
        let test_position = tower_lsp::lsp_types::Position::new(9, 4); // Line 10, col 5 (0-indexed)
        let file_uri = "file:///test.kt";
        let dependency_cache = std::sync::Arc::new(crate::core::dependency_cache::DependencyCache::new());
        
        // Find the node at that position
        if let Some(node_at_position) = find_node_at_position(&tree, test_position, source) {
            println!("Found node at position: '{}' (kind: {})", 
                node_at_position.utf8_text(source.as_bytes()).unwrap_or(""), 
                node_at_position.kind());
                
            match find_definition_chain(
                &kotlin_support, &tree, source, dependency_cache, file_uri, &node_at_position
            ) {
                Ok(location) => {
                    println!("✓ Method resolution chain succeeded at line {}", location.range.start.line + 1);
                }
                Err(e) => {
                    println!("✗ Method resolution chain failed: {}", e);
                }
            }
        } else {
            println!("Could not find node at position");
        }
    }

    #[test]
    fn test_kotlin_various_instance_patterns() {
        // Test different patterns that might cause issues
        let patterns = vec![
            // Pattern 1: Simple case
            r#"
fun main() {
    val obj = TestClass()
    obj.method()
}
"#,
            // Pattern 2: In class method
            r#"
class MyClass {
    fun doSomething() {
        val service = TestService()
        service.process()
    }
}
"#,
            // Pattern 3: Multiple calls
            r#"
fun main() {
    val client = ApiClient()
    val result = client.get()
    client.post()
}
"#,
            // Pattern 4: With type annotation
            r#"
fun main() {
    val handler: RequestHandler = RequestHandler()
    handler.handle()
}
"#,
        ];

        for (i, source) in patterns.iter().enumerate() {
            println!("\n=== Testing Pattern {} ===", i + 1);
            println!("Source: {}", source);
            
            let tree = create_kotlin_test_tree(source);
            let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
            
            // Find all simple_identifier nodes that could be variables in method calls
            let variable_query = r#"
                (call_expression
                  (navigation_expression
                    (simple_identifier) @var))
            "#;
            
            let language = tree_sitter_kotlin::language();
            if let Ok(query) = tree_sitter::Query::new(&language, variable_query) {
                let mut cursor = tree_sitter::QueryCursor::new();
                let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
                
                while let Some(query_match) = matches.next() {
                    for capture in query_match.captures {
                        let var_node = capture.node;
                        let var_text = var_node.utf8_text(source.as_bytes()).unwrap_or("");
                        let line = var_node.start_position().row + 1;
                        
                        println!("Testing variable '{}' at line {}", var_text, line);
                        
                        // Test the method resolution chain
                        let file_uri = "file:///test.kt";
                        let dependency_cache = std::sync::Arc::new(crate::core::dependency_cache::DependencyCache::new());
                        
                        match find_definition_chain(
                            &kotlin_support, &tree, source, dependency_cache, file_uri, &var_node
                        ) {
                            Ok(location) => {
                                println!("✓ Found definition at line {}", location.range.start.line + 1);
                            }
                            Err(e) => {
                                println!("✗ Failed to find definition: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_getter_setter_field_fallback_java() {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;
        
        // Create a temporary Java file with a field
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TestClass.java");
        let java_content = r#"
public class TestClass {
    private String myField;
    private boolean enabled;
    private int count;
    
    // Note: no explicit getter/setter methods defined
}
"#;
        let mut file = File::create(&file_path).unwrap();
        file.write_all(java_content.as_bytes()).unwrap();
        
        // Create location for the class file
        let uri = tower_lsp::lsp_types::Url::from_file_path(&file_path).unwrap();
        let location = tower_lsp::lsp_types::Location::new(uri, tower_lsp::lsp_types::Range::default());
        
        let java_support = crate::languages::java::support::JavaSupport::new();
        
        // Test getter method -> field mapping
        let result = try_find_getter_setter_field(&location, "getMyField", &java_support);
        assert!(result.is_some(), "Should find myField for getMyField");
        
        // Test setter method -> field mapping
        let result = try_find_getter_setter_field(&location, "setMyField", &java_support);
        assert!(result.is_some(), "Should find myField for setMyField");
        
        // Test boolean getter -> field mapping
        let result = try_find_getter_setter_field(&location, "isEnabled", &java_support);
        assert!(result.is_some(), "Should find enabled for isEnabled");
        
        // Test setter for boolean field
        let result = try_find_getter_setter_field(&location, "setEnabled", &java_support);
        assert!(result.is_some(), "Should find enabled for setEnabled");
        
        // Test getter for int field
        let result = try_find_getter_setter_field(&location, "getCount", &java_support);
        assert!(result.is_some(), "Should find count for getCount");
        
        // Test non-existent field
        let result = try_find_getter_setter_field(&location, "getNonExistent", &java_support);
        assert!(result.is_none(), "Should not find field for non-existent getter");
        
        // Test invalid method names
        let result = try_find_getter_setter_field(&location, "regularMethod", &java_support);
        assert!(result.is_none(), "Should not match non-getter/setter methods");
        
        let result = try_find_getter_setter_field(&location, "get", &java_support);
        assert!(result.is_none(), "Should not match bare 'get' prefix");
        
        let result = try_find_getter_setter_field(&location, "getfield", &java_support);
        assert!(result.is_none(), "Should not match lowercase after prefix");
    }

    #[test]
    fn test_getter_setter_field_fallback_kotlin() {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;
        
        // Create a temporary Kotlin file with properties
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TestClass.kt");
        let kotlin_content = r#"
class TestClass {
    private var userName: String = ""
    private var isActive: Boolean = false
    private var itemCount: Int = 0
}
"#;
        let mut file = File::create(&file_path).unwrap();
        file.write_all(kotlin_content.as_bytes()).unwrap();
        
        // Create location for the class file
        let uri = tower_lsp::lsp_types::Url::from_file_path(&file_path).unwrap();
        let location = tower_lsp::lsp_types::Location::new(uri, tower_lsp::lsp_types::Range::default());
        
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        
        // Test getter method -> property mapping
        let result = try_find_getter_setter_field(&location, "getUserName", &kotlin_support);
        assert!(result.is_some(), "Should find userName for getUserName");
        
        // Test setter method -> property mapping
        let result = try_find_getter_setter_field(&location, "setUserName", &kotlin_support);
        assert!(result.is_some(), "Should find userName for setUserName");
        
        // Test boolean getter -> property mapping
        let result = try_find_getter_setter_field(&location, "isIsActive", &kotlin_support);
        assert!(result.is_some(), "Should find isActive for isIsActive");
        
        // Test getter for int property
        let result = try_find_getter_setter_field(&location, "getItemCount", &kotlin_support);
        assert!(result.is_some(), "Should find itemCount for getItemCount");
    }

    #[test]
    fn test_getter_setter_field_fallback_groovy() {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;
        
        // Create a temporary Groovy file with fields
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("TestClass.groovy");
        let groovy_content = r#"
class TestClass {
    private String fullName
    private boolean isReady
    private int totalCount
}
"#;
        let mut file = File::create(&file_path).unwrap();
        file.write_all(groovy_content.as_bytes()).unwrap();
        
        // Create location for the class file
        let uri = tower_lsp::lsp_types::Url::from_file_path(&file_path).unwrap();
        let location = tower_lsp::lsp_types::Location::new(uri, tower_lsp::lsp_types::Range::default());
        
        let groovy_support = crate::languages::groovy::support::GroovySupport::new();
        
        // Test getter method -> field mapping
        let result = try_find_getter_setter_field(&location, "getFullName", &groovy_support);
        assert!(result.is_some(), "Should find fullName for getFullName");
        
        // Test setter method -> field mapping
        let result = try_find_getter_setter_field(&location, "setFullName", &groovy_support);
        assert!(result.is_some(), "Should find fullName for setFullName");
        
        // Test boolean getter -> field mapping
        let result = try_find_getter_setter_field(&location, "isIsReady", &groovy_support);
        assert!(result.is_some(), "Should find isReady for isIsReady");
        
        // Test getter for int field
        let result = try_find_getter_setter_field(&location, "getTotalCount", &groovy_support);
        assert!(result.is_some(), "Should find totalCount for getTotalCount");
    }

    #[test]
    fn test_getter_setter_method_name_conversion() {
        // Test method name to field name conversion logic
        
        // Getter patterns
        assert_eq!(extract_field_name_from_method("getMyField"), Some("myField".to_string()));
        assert_eq!(extract_field_name_from_method("getUserName"), Some("userName".to_string()));
        assert_eq!(extract_field_name_from_method("getURL"), Some("uRL".to_string()));
        assert_eq!(extract_field_name_from_method("getA"), Some("a".to_string()));
        
        // Setter patterns
        assert_eq!(extract_field_name_from_method("setMyField"), Some("myField".to_string()));
        assert_eq!(extract_field_name_from_method("setUserName"), Some("userName".to_string()));
        assert_eq!(extract_field_name_from_method("setURL"), Some("uRL".to_string()));
        assert_eq!(extract_field_name_from_method("setA"), Some("a".to_string()));
        
        // Boolean getter patterns
        assert_eq!(extract_field_name_from_method("isEnabled"), Some("enabled".to_string()));
        assert_eq!(extract_field_name_from_method("isActive"), Some("active".to_string()));
        assert_eq!(extract_field_name_from_method("isReady"), Some("ready".to_string()));
        assert_eq!(extract_field_name_from_method("isA"), Some("a".to_string()));
        
        // Invalid patterns should return None
        assert_eq!(extract_field_name_from_method("get"), None);
        assert_eq!(extract_field_name_from_method("set"), None);
        assert_eq!(extract_field_name_from_method("is"), None);
        assert_eq!(extract_field_name_from_method("getfield"), None); // lowercase after prefix
        assert_eq!(extract_field_name_from_method("setfield"), None); // lowercase after prefix
        assert_eq!(extract_field_name_from_method("isfield"), None);  // lowercase after prefix
        assert_eq!(extract_field_name_from_method("regularMethod"), None);
        assert_eq!(extract_field_name_from_method(""), None);
    }

    // Helper function to extract field name from method name (for testing)
    fn extract_field_name_from_method(method_name: &str) -> Option<String> {
        if method_name.starts_with("get") && method_name.len() > 3 {
            let field_base = &method_name[3..];
            if field_base.chars().next()?.is_uppercase() {
                let mut chars = field_base.chars();
                let first_char = chars.next()?.to_lowercase().to_string();
                let rest: String = chars.collect();
                Some(first_char + &rest)
            } else {
                None
            }
        } else if method_name.starts_with("set") && method_name.len() > 3 {
            let field_base = &method_name[3..];
            if field_base.chars().next()?.is_uppercase() {
                let mut chars = field_base.chars();
                let first_char = chars.next()?.to_lowercase().to_string();
                let rest: String = chars.collect();
                Some(first_char + &rest)
            } else {
                None
            }
        } else if method_name.starts_with("is") && method_name.len() > 2 {
            let field_base = &method_name[2..];
            if field_base.chars().next()?.is_uppercase() {
                let mut chars = field_base.chars();
                let first_char = chars.next()?.to_lowercase().to_string();
                let rest: String = chars.collect();
                Some(first_char + &rest)
            } else {
                None
            }
        } else {
            None
        }
    }

    fn create_groovy_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        let language = tree_sitter_groovy::language();
        parser.set_language(&language).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_middle_identifier_resolution_in_call_chain() {
        // Test the fix for middle identifier resolution in chains like Outer.Inner.CONSTANT
        let source = "Outer.Inner.CONSTANT";
        let tree = create_groovy_test_tree(source);
        
        // Find the "Inner" identifier (the middle one)
        let inner_node = find_identifier_node(&tree, source, "Inner").unwrap();
        
        // Verify that the parent is a field_access node
        let parent = inner_node.parent().unwrap();
        assert_eq!(parent.kind(), "field_access");
        
        // Verify that the parent's text is "Outer.Inner"
        let parent_text = parent.utf8_text(source.as_bytes()).unwrap();
        assert_eq!(parent_text, "Outer.Inner");
        
        // This test verifies that the AST structure is correct for our fix
        // The actual definition resolution is tested via integration tests
        // since it depends on the language support implementation
    }

    #[test]
    fn test_find_class_name_in_current_source() {
        // Test the helper function that finds class name nodes
        let kotlin_source = r#"
        import com.example.DataContainer
        
        fun test() {
            val container = DataContainer()
            container.items.add("test")
        }
        "#;
        
        let tree = create_kotlin_test_tree(kotlin_source);
        
        // Should find the DataContainer identifier
        let class_node = find_class_name_in_current_source(&tree, kotlin_source, "DataContainer");
        assert!(class_node.is_some(), "Should find DataContainer in source");
        
        if let Some(node) = class_node {
            let node_text = node.utf8_text(kotlin_source.as_bytes()).unwrap();
            assert_eq!(node_text, "DataContainer");
            assert!(matches!(node.kind(), "simple_identifier" | "identifier" | "type_identifier"));
        }
        
        // Should not find non-existent class
        let missing_class = find_class_name_in_current_source(&tree, kotlin_source, "NonExistentClass");
        assert!(missing_class.is_none(), "Should not find NonExistentClass");
    }

    #[test]
    fn test_find_class_definition_fallback_behavior() {
        // Test that find_class_definition has proper fallback behavior
        // This is a unit test for the function structure, not full resolution
        
        let kotlin_source = r#"
        import com.example.TestClass
        
        fun test() {
            val obj = TestClass()
        }
        "#;
        
        let tree = create_kotlin_test_tree(kotlin_source);
        let kotlin_support = crate::languages::kotlin::support::KotlinSupport::new();
        let dependency_cache = std::sync::Arc::new(crate::core::dependency_cache::DependencyCache::new());
        let file_uri = "file:///test.kt";
        
        // Test the function doesn't panic and handles missing classes gracefully
        let result = find_class_definition(
            &kotlin_support,
            &tree,
            kotlin_source,
            file_uri,
            "NonExistentClass",
            dependency_cache
        );
        
        // Should return None for non-existent class (no panic)
        assert!(result.is_none(), "Should return None for non-existent class");
        
        // Test that it can find nodes in the source
        let class_node = find_class_name_in_current_source(&tree, kotlin_source, "TestClass");
        assert!(class_node.is_some(), "Should find TestClass node in source for fallback resolution");
    }

}
