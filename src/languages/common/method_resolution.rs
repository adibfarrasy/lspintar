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
pub fn extract_static_method_context(usage_node: &Node, source: &str) -> Option<(String, String)> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    debug!(
        "extract_static_method_context: analyzing node '{}' of kind '{}'",
        usage_text,
        usage_node.kind()
    );

    let method_invocation = find_parent_method_invocation_node(usage_node);
    if method_invocation.is_none() {
        debug!("extract_static_method_context: no method_invocation parent found");
        return None;
    }
    let method_invocation = method_invocation.unwrap();
    debug!("extract_static_method_context: found method_invocation parent of kind '{}'", method_invocation.kind());

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
            debug!("extract_static_method_context: usage_node matches method name - static method call detected");
            Some((class_name, method_name))
        } else if usage_text == class_name {
            debug!("extract_static_method_context: usage_node matches class name - returning static method context anyway");
            Some((class_name, method_name))
        } else {
            debug!("extract_static_method_context: usage_node '{}' matches neither class '{}' nor method '{}'", 
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
        debug!("extract_instance_method_context: no method_invocation parent found");
        return None;
    }
    let method_invocation = method_invocation.unwrap();
    debug!("extract_instance_method_context: found method_invocation parent");

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
            debug!("extract_instance_method_context: usage_node matches method name - instance method call detected");
            Some((variable_name, method_name))
        } else if usage_text == variable_name {
            debug!("extract_instance_method_context: usage_node matches variable name - returning instance method context anyway");
            Some((variable_name, method_name))
        } else {
            debug!("extract_instance_method_context: usage_node '{}' matches neither variable '{}' nor method '{}'", 
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

/// Common logic for finding static method definitions
pub fn find_static_method_definition(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    class_name: &str,
    method_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!(
        "find_static_method_definition: looking for {}.{}",
        class_name, method_name
    );

    // Create a temporary node representing the class name for resolution
    // This is needed because the existing resolution methods expect a usage_node
    // For now, we'll use the existing usage_node but this should be improved
    
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
            debug!("find_static_method_definition: found method {} in class file", method_name);
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

/// Try to resolve a class name using various resolution strategies
fn try_resolve_class_name(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    class_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // Strategy 1: Try to find class locally (same file) - rarely works for static methods
    // but worth trying for inner classes
    
    // Strategy 2: Try project-level resolution using import resolution
    // This is where we need to create a proper symbol lookup key
    if let Some(location) = try_resolve_class_via_projects(
        language_support, source, file_uri, class_name, dependency_cache.clone()
    ) {
        return Some(location);
    }
    
    // Strategy 3: Try workspace resolution
    if let Some(location) = try_resolve_class_via_workspace(
        language_support, source, file_uri, class_name, dependency_cache.clone()
    ) {
        return Some(location);
    }
    
    // Strategy 4: Try external dependencies
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
        Err(e) => {
            return None;
        }
    };
    
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    let mut import_count = 0;
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                import_count += 1;
                
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

/// Split FQN into package and class parts for cache lookup
fn split_fqn(fqn: &str) -> Option<(String, String)> {
    if let Some(last_dot) = fqn.rfind('.') {
        let package_part = &fqn[..last_dot];
        let class_part = &fqn[last_dot + 1..];
        Some((package_part.to_string(), class_part.to_string()))
    } else {
        // No package, just class name
        Some(("".to_string(), fqn.to_string()))
    }
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
    // TODO: Implement workspace-level class resolution
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
    // TODO: Implement external dependency class resolution
    None
}

/// Search for a method within a class file, detecting the target language automatically
pub fn search_method_in_class_file_cross_language(
    class_location: &Location,
    method_name: &str,
) -> Option<Location> {
    
    // Determine the language from the file extension
    let target_language_support = detect_language_from_uri(class_location.uri.as_str())?;
    
    // Use the target language's method search logic
    search_method_in_class_file(class_location, method_name, target_language_support.as_ref())
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
fn search_method_in_class_file(
    class_location: &Location,
    method_name: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    debug!(
        "METHODRES: search_method_in_class_file: searching for method {} in {} using {}",
        method_name, class_location.uri, language_support.language_id()
    );
    
    // Extract file path from URI and read content
    let file_path = class_location.uri.to_file_path().ok()?;
    let content = std::fs::read_to_string(&file_path).ok()?;
    
    // Parse the target file with the appropriate language parser
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
pub use crate::languages::groovy::definition::method_resolution::{
    CallSignature, 
    extract_call_signature_from_context
};


/// Common logic for finding instance method definitions  
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
    debug!(
        "METHODRES: find_instance_method_definition: looking for {}.{}",
        variable_name, method_name
    );

    // Step 1: Extract the variable type using language-specific trait methods
    let variable_type = match extract_variable_type_from_tree(language_support, variable_name, tree, source, usage_node) {
        Some(t) => t,
        None => {
            return None;
        }
    };
    
    debug!(
        "METHODRES: find_instance_method_definition: variable {} has type {}",
        variable_name, variable_type
    );
    
    // Step 2: Find the class definition for the variable type
    let class_location = find_class_definition(
        language_support, tree, source, file_uri, &variable_type, dependency_cache.clone()
    )?;
    
    debug!(
        "METHODRES: find_instance_method_definition: found type class {} at {:?}",
        variable_type, class_location.uri
    );
    
    // Step 3: Extract call signature for overload resolution
    let call_signature = extract_call_signature_from_context(usage_node, source);
    
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

/// Find class definition by trying all supported languages
fn find_class_definition(
    _language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    class_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    
    // Find any occurrence of the class name in the source to get a node
    if let Some(class_node) = find_class_name_node(tree, source, class_name) {
        
        // Convert to LSP position
        let type_position = node_to_lsp_position(&class_node);
        
        // Create all language supports and try each one
        let language_supports: Vec<Box<dyn LanguageSupport + Send + Sync>> = 
            crate::languages::ALL_LANGUAGE_SUPPORTS.iter().map(|f| f()).collect();
        
        // Try each language support's find_definition method
        for (i, lang_support) in language_supports.iter().enumerate() {
            
            match lang_support.find_definition(tree, source, type_position, file_uri, dependency_cache.clone()) {
                Ok(location) => {
                    return Some(location);
                }
                Err(e) => {
                }
            }
        }
    } else {
    }
    
    None
}

/// Convert tree-sitter node to LSP position
fn node_to_lsp_position(node: &tree_sitter::Node) -> tower_lsp::lsp_types::Position {
    let start_pos = node.start_position();
    tower_lsp::lsp_types::Position {
        line: start_pos.row as u32,
        character: start_pos.column as u32,
    }
}

/// Find a node in the tree that represents the given class name
fn find_class_name_node<'a>(tree: &'a Tree, source: &'a str, class_name: &str) -> Option<tree_sitter::Node<'a>> {
    
    // Simple approach: find any identifier node that matches the class name
    fn search_node<'a>(node: tree_sitter::Node<'a>, source: &'a str, target: &str) -> Option<tree_sitter::Node<'a>> {
        // Check if this node is an identifier with the target text
        if node.kind() == "identifier" || node.kind() == "type_identifier" {
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
    
    let result = search_node(tree.root_node(), source, class_name);
    if result.is_some() {
    } else {
    }
    result
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


/// Find variable or field declaration by name
fn find_variable_or_field_declaration(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    file_uri: &str,
    variable_name: &str,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!("find_variable_or_field_declaration: looking for '{}'", variable_name);
    
    // First, try to find as a local variable or parameter
    if let Some(location) = find_local_variable_by_name(tree, source, file_uri, variable_name) {
        debug!("find_variable_or_field_declaration: found as local variable");
        return Some(location);
    }
    
    // Next, try to find as a class field in the current file
    if let Some(location) = find_field_in_current_file(tree, source, file_uri, variable_name) {
        debug!("find_variable_or_field_declaration: found as field in current file");
        return Some(location);
    }
    
    // Could also be in parent class or imported - for now we'll focus on current file
    debug!("find_variable_or_field_declaration: not found");
    None
}

/// Find a local variable by name in the current file
fn find_local_variable_by_name(tree: &Tree, source: &str, file_uri: &str, variable_name: &str) -> Option<Location> {
    use tower_lsp::lsp_types::{Position, Range, Url};
    
    // Search for local variable declarations with this name
    let query_text = r#"
        (local_variable_declaration
          declarator: (variable_declarator
            name: (identifier) @var_name))
        (parameter
          name: (identifier) @param_name)
    "#;
    
    let language = tree.language();
    let query = tree_sitter::Query::new(&language, query_text).ok()?;
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                if text == variable_name {
                    let range = capture.node.range();
                    let start = Position::new(range.start_point.row as u32, range.start_point.column as u32);
                    let end = Position::new(range.end_point.row as u32, range.end_point.column as u32);
                    let uri = Url::parse(file_uri).ok()?;
                    return Some(Location::new(uri, Range::new(start, end)));
                }
            }
        }
    }
    
    None
}

/// Find a field in the current file
fn find_field_in_current_file(tree: &Tree, source: &str, file_uri: &str, field_name: &str) -> Option<Location> {
    use tower_lsp::lsp_types::{Position, Range, Url};
    
    debug!("find_field_in_current_file: searching for field '{}'", field_name);
    
    // Search for field declarations with this name
    // This query works for Java/Groovy
    let query_text = r#"
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_name))
    "#;
    
    let language = tree.language();
    let query = match tree_sitter::Query::new(&language, query_text) {
        Ok(q) => q,
        Err(e) => {
            debug!("find_field_in_current_file: failed to create query: {:?}", e);
            return None;
        }
    };
    
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                debug!("find_field_in_current_file: checking field '{}' against '{}'", text, field_name);
                if text == field_name {
                    debug!("find_field_in_current_file: found matching field!");
                    let range = capture.node.range();
                    let start = Position::new(range.start_point.row as u32, range.start_point.column as u32);
                    let end = Position::new(range.end_point.row as u32, range.end_point.column as u32);
                    let uri = Url::parse(file_uri).ok()?;
                    return Some(Location::new(uri, Range::new(start, end)));
                }
            }
        }
    }
    
    debug!("find_field_in_current_file: field '{}' not found", field_name);
    None
}

/// Extract type from source text as a fallback
fn extract_type_from_source_text(location: &Location, variable_name: &str, source: &str) -> Option<String> {
    debug!("extract_type_from_source_text: attempting regex-based extraction for {}", variable_name);
    
    // Find the line in the source that contains the declaration
    let lines: Vec<&str> = source.lines().collect();
    let line_num = location.range.start.line as usize;
    
    if line_num >= lines.len() {
        return None;
    }
    
    // Look at a few lines around the location to find the type
    let start = if line_num > 2 { line_num - 2 } else { 0 };
    let end = std::cmp::min(line_num + 3, lines.len());
    
    for i in start..end {
        let line = lines[i];
        // Look for patterns like "TypeName variableName" or "@Inject TypeName variableName"
        if line.contains(variable_name) {
            // Try to extract type using simple pattern matching
            let trimmed = line.trim();
            
            // Remove annotations
            let without_annotations = trimmed.split('@')
                .last()
                .unwrap_or(trimmed)
                .trim();
            
            // Look for the variable name and extract what comes before it
            if let Some(var_pos) = without_annotations.find(variable_name) {
                let before_var = &without_annotations[..var_pos].trim();
                // Get the last word before the variable name (that's likely the type)
                if let Some(type_name) = before_var.split_whitespace().last() {
                    if !is_modifier(type_name) && type_name != variable_name {
                        debug!("extract_type_from_source_text: extracted type '{}' from line: {}", type_name, line);
                        return Some(type_name.to_string());
                    }
                }
            }
        }
    }
    
    debug!("extract_type_from_source_text: could not extract type for {}", variable_name);
    None
}

/// Extract the type of a variable from its declaration location
fn extract_variable_type(variable_location: &Location, variable_name: &str, language_support: &dyn LanguageSupport) -> Option<String> {
    debug!(
        "extract_variable_type: extracting type for variable {} from {:?}",
        variable_name, variable_location.uri
    );
    
    // Read the file and find the variable declaration
    use std::fs;
    let file_path = variable_location.uri.to_file_path().ok()?;
    let content = fs::read_to_string(&file_path).ok()?;
    
    // Parse the file with tree-sitter
    let mut parser = language_support.create_parser();
    let tree = parser.parse(&content, None)?;
    
    // Find the variable declaration node at the given location
    let start_line = variable_location.range.start.line as usize;
    let start_col = variable_location.range.start.character as usize;
    
    // Find the node at the declaration position
    let root = tree.root_node();
    let declaration_node = find_node_at_position(root, start_line, start_col)?;
    
    debug!("extract_variable_type: found declaration node of kind: {}", declaration_node.kind());
    
    // Try to extract type using language-specific patterns
    extract_type_from_declaration_node(&declaration_node, variable_name, &content)
}

/// Find node at a specific position in the tree
fn find_node_at_position(node: Node, line: usize, col: usize) -> Option<Node> {
    let point = tree_sitter::Point::new(line, col);
    
    // Check if this node contains the position
    if !(node.range().start_point <= point && point <= node.range().end_point) {
        return None;
    }
    
    // Try to find the most specific child
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(found) = find_node_at_position(child, line, col) {
                return Some(found);
            }
        }
    }
    
    // This node itself is the most specific
    Some(node)
}

/// Extract type from a declaration node using tree-sitter
fn extract_type_from_declaration_node(node: &Node, variable_name: &str, source: &str) -> Option<String> {
    debug!("extract_type_from_declaration_node: examining node of kind '{}' for variable '{}'", node.kind(), variable_name);
    
    // If we're starting from an identifier, walk up to find the declaration
    let mut current = *node;
    if current.kind() == "identifier" {
        // Walk up to find the actual declaration node
        while let Some(parent) = current.parent() {
            let parent_kind = parent.kind();
            debug!("extract_type_from_declaration_node: walking up, parent kind: {}", parent_kind);
            if parent_kind == "field_declaration" || parent_kind == "local_variable_declaration" || 
               parent_kind == "property_declaration" || parent_kind == "variable_declaration" {
                current = parent;
                break;
            }
            current = parent;
        }
    }
    
    // Now extract type from the declaration node
    loop {
        let kind = current.kind();
        debug!("extract_type_from_declaration_node: processing node kind: {}", kind);
        
        // Java/Groovy patterns
        if kind == "field_declaration" || kind == "local_variable_declaration" {
            debug!("extract_type_from_declaration_node: found declaration node, extracting type");
            
            // Look for type child
            if let Some(type_node) = current.child_by_field_name("type") {
                if let Ok(type_text) = type_node.utf8_text(source.as_bytes()) {
                    debug!("extract_type_from_declaration_node: found type via 'type' field: {}", type_text);
                    // Clean up the type (remove annotations, etc.)
                    let clean_type = type_text.split_whitespace().last().unwrap_or(type_text);
                    return Some(clean_type.to_string());
                }
            }
            
            // Also check direct children for type information
            for i in 0..current.child_count() {
                if let Some(child) = current.child(i) {
                    let child_kind = child.kind();
                    debug!("extract_type_from_declaration_node: checking child {} of kind '{}'", i, child_kind);
                    
                    // Skip modifiers and annotations
                    if child_kind == "modifiers" || child_kind == "marker_annotation" || child_kind == "annotation" {
                        continue;
                    }
                    
                    if child_kind == "type_identifier" || child_kind == "simple_type" || child_kind == "scoped_type_identifier" {
                        if let Ok(type_text) = child.utf8_text(source.as_bytes()) {
                            debug!("extract_type_from_declaration_node: found type from child: {}", type_text);
                            return Some(type_text.to_string());
                        }
                    }
                    
                    // For generic types, get the base type
                    if child_kind == "generic_type" {
                        // First try the whole generic type
                        if let Some(base_type) = child.child(0) {
                            if base_type.kind() == "type_identifier" || base_type.kind() == "scoped_type_identifier" {
                                if let Ok(type_text) = base_type.utf8_text(source.as_bytes()) {
                                    debug!("extract_type_from_declaration_node: found generic base type: {}", type_text);
                                    return Some(type_text.to_string());
                                }
                            }
                        }
                    }
                    
                    // Sometimes the type is nested deeper
                    if child_kind == "variable_declarator" {
                        // Skip the declarator, we want the type that comes before it
                        continue;
                    }
                }
            }
            
            // If we still haven't found the type, log what we have
            debug!("extract_type_from_declaration_node: could not extract type from field_declaration");
            if let Ok(decl_text) = current.utf8_text(source.as_bytes()) {
                debug!("extract_type_from_declaration_node: full declaration text: {}", decl_text);
            }
        }
        
        // Kotlin patterns
        if kind == "property_declaration" || kind == "variable_declaration" {
            // Look for user_type child
            for i in 0..current.child_count() {
                if let Some(child) = current.child(i) {
                    if child.kind() == "user_type" {
                        if let Some(type_id) = child.child_by_field_name("type_identifier")
                            .or_else(|| child.child(0)) {
                            if let Ok(type_text) = type_id.utf8_text(source.as_bytes()) {
                                debug!("extract_type_from_declaration_node: found Kotlin type: {}", type_text);
                                return Some(type_text.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        // Move up to parent
        if let Some(parent) = current.parent() {
            current = parent;
        } else {
            break;
        }
    }
    
    debug!("extract_type_from_declaration_node: could not extract type for variable {}", variable_name);
    None
}

/// Extract type name from a variable declaration line (fallback text-based approach)
fn extract_type_from_declaration(line: &str, variable_name: &str) -> Option<String> {
    let trimmed = line.trim();
    
    // Pattern 1: "Type variableName = ..." (Java/Groovy style)
    // Pattern 2: "val variableName: Type = ..." (Kotlin style) 
    // Pattern 3: "var variableName: Type = ..." (Kotlin style)
    
    // Java/Groovy pattern: look for "Type variableName"
    if let Some(var_pos) = trimmed.find(&format!(" {}", variable_name)) {
        let before_var = &trimmed[..var_pos].trim();
        
        // Split by whitespace and take the last token as the type
        let tokens: Vec<&str> = before_var.split_whitespace().collect();
        if let Some(type_token) = tokens.last() {
            // Filter out modifiers like public, private, static, final, etc.
            if !is_modifier(type_token) {
                return Some(type_token.to_string());
            }
            
            // Look for the token before modifiers
            for token in tokens.iter().rev() {
                if !is_modifier(token) {
                    return Some(token.to_string());
                }
            }
        }
    }
    
    // Kotlin pattern: "val/var variableName: Type"
    if trimmed.contains(&format!("{}: ", variable_name)) {
        if let Some(colon_pos) = trimmed.find(&format!("{}: ", variable_name)) {
            let after_colon = &trimmed[colon_pos + variable_name.len() + 2..];
            
            // Extract type name (up to = or whitespace)
            let type_end = after_colon.find('=').unwrap_or(after_colon.len());
            let type_part = after_colon[..type_end].trim();
            
            if !type_part.is_empty() {
                return Some(type_part.to_string());
            }
        }
    }
    
    None
}

/// Check if a token is a language modifier
fn is_modifier(token: &str) -> bool {
    matches!(token, "public" | "private" | "protected" | "static" | "final" | "abstract" |
                   "synchronized" | "volatile" | "transient" | "native" | "strictfp" |
                   "val" | "var" | "const" | "lateinit" | "open" | "override" | "inner" |
                   "sealed" | "data" | "enum" | "annotation")
}

/// Common enhanced find_definition_chain that handles method resolution
pub fn find_definition_chain_with_method_resolution(
    language_support: &dyn LanguageSupport,
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
) -> Result<Location, anyhow::Error> {
    let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    debug!(
        "find_definition_chain_with_method_resolution: starting resolution for '{}' (kind: {})",
        usage_text, usage_node.kind()
    );

    // Try static method resolution first
    if let Some((class_name, method_name)) = language_support.extract_static_method_context(usage_node, source) {
        debug!(
            "find_definition_chain_with_method_resolution: detected static method call {}.{}",
            class_name, method_name
        );
        if let Some(location) = find_static_method_definition(
            language_support, tree, source, file_uri, usage_node, &class_name, &method_name, dependency_cache.clone()
        ) {
            debug!("find_definition_chain_with_method_resolution: static method resolution succeeded");
            return Ok(location);
        }
        debug!("find_definition_chain_with_method_resolution: static method resolution failed, continuing");
    } else {
        debug!("find_definition_chain_with_method_resolution: no static method context detected");
    }

    // Try instance method resolution  
    if let Some((variable_name, method_name)) = language_support.extract_instance_method_context(usage_node, source) {
        if let Some(location) = language_support.find_instance_method_definition(
            tree, source, file_uri, usage_node, &variable_name, &method_name, dependency_cache.clone()
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
                            if let Some(final_location) = language_support.set_start_position(source, usage_node, &uri_string) {
                                return Ok(final_location);
                            }
                        }
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
        .or_else(|| language_support.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone()))
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
                    language_support.set_start_position(source, usage_node, &uri_string)
                }
            }
        })
        .ok_or_else(|| anyhow::anyhow!("Definition not found"))
}