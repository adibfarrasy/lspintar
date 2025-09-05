use anyhow::Result;
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::task;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Tree, StreamingIterator};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::path_to_file_uri,
    },
    languages::LanguageSupport,
};

use super::utils::find_identifier_at_position;

#[tracing::instrument(skip_all)]
pub fn handle(
    tree: &Tree,
    source: &str,
    position: Position,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Result<Vec<Location>> {
    let identifier_node = find_identifier_at_position(tree, source, position)
        .ok_or_else(|| anyhow::anyhow!("Could not find identifier at position"))?;
    let symbol_name = identifier_node.utf8_text(source.as_bytes())?;
    let symbol_type =
        language_support.determine_symbol_type_from_context(tree, &identifier_node, source)?;

    match symbol_type {
        SymbolType::InterfaceDeclaration | SymbolType::ClassDeclaration | SymbolType::Type => {
            // Find all implementations of this interface/class
            futures::executor::block_on(find_implementations(symbol_name, &dependency_cache))
        }
        SymbolType::MethodCall => {
            // Find the method declaration and then its implementations
            handle_method_call_implementation(tree, source, position, dependency_cache)
        }
        SymbolType::MethodDeclaration => {
            // Find implementations of this method (if it's in an interface or abstract class)
            futures::executor::block_on(find_method_implementations(tree, source, symbol_name, &dependency_cache))
        }
        _ => {
            // For other symbol types, return empty result
            Ok(vec![])
        }
    }
}

#[tracing::instrument(skip_all)]
async fn find_implementations(
    interface_name: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    // First, try to get project roots from existing in-memory data
    let mut project_roots: HashSet<PathBuf> = dependency_cache
        .inheritance_index
        .iter()
        .map(|entry| entry.key().0.clone())
        .collect();

    // Get additional project roots from symbol_index if inheritance_index is sparse
    if project_roots.is_empty() {
        project_roots = dependency_cache
            .symbol_index
            .iter()
            .map(|entry| entry.key().0.clone())
            .collect();
    }

    let mut locations = Vec::new();
    let mut tasks = Vec::new();

    for project_root in project_roots {
        let dependency_cache_clone = Arc::clone(dependency_cache);
        let interface_name_clone = interface_name.to_string();

        let task = task::spawn(async move {
            dependency_cache_clone
                .find_inheritance_implementations(&project_root, &interface_name_clone)
                .await
        });

        tasks.push(task);
    }

    // Wait for all tasks to complete
    for task in tasks {
        if let Some(project_locations) = task.await? {
            // Convert (PathBuf, usize, usize) to Location
            for (file_path, start_line, start_col) in project_locations {
                if let Some(file_uri) = path_to_file_uri(&file_path) {
                    let location = Location {
                        uri: Url::parse(&file_uri).unwrap_or_else(|_| Url::parse("file:///").unwrap()),
                        range: Range::new(
                            Position::new(start_line as u32, start_col as u32),
                            Position::new(start_line as u32, start_col as u32 + 1),
                        ),
                    };
                    locations.push(location);
                }
            }
        }
    }

    Ok(locations)
}

#[tracing::instrument(skip_all)]
fn handle_method_call_implementation(
    tree: &Tree,
    source: &str,
    position: Position,
    dependency_cache: Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    let identifier_node = find_identifier_at_position(tree, source, position)
        .ok_or_else(|| anyhow::anyhow!("Could not find identifier at position"))?;
    
    // Check if this is an instance method call and get the variable/method info
    let instance_context = extract_instance_method_context(&identifier_node, source);
    
    if let Some((variable_name, method_name)) = instance_context {
        // Resolve the variable type to get the interface/class name  
        let variable_type = resolve_variable_type(&variable_name, tree, source, &identifier_node);
        
        if let Some(class_name) = variable_type {
            // Find implementations of this class/interface and look for the method
            return futures::executor::block_on(find_interface_method_implementations(
                &class_name,
                &method_name,
                &dependency_cache
            ));
        } else {
            return Err(anyhow::anyhow!("Cannot resolve variable type for go-to-implementation"));
        }
    } else {
        return Err(anyhow::anyhow!("Go-to-implementation only supports instance method calls"));
    }
}

#[tracing::instrument(skip_all)]
async fn find_method_implementations(
    tree: &Tree,
    source: &str,
    method_name: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    // First, find the interface/class that contains this method
    let parent_name = get_parent_name(tree, source, method_name)
        .ok_or_else(|| anyhow::anyhow!("Could not find parent class/interface for method {}", method_name))?;

    // Find all implementations of this interface/class
    let interface_implementations = find_implementations(&parent_name, dependency_cache).await?;
    
    let mut method_implementations = Vec::new();
    
    // For each implementation, look for the specific method
    for implementation_location in interface_implementations {
        if let Some(method_location) = find_method_in_class(&implementation_location, method_name).await? {
            method_implementations.push(method_location);
        }
    }
    
    Ok(method_implementations)
}

/// Extract instance method context from an identifier node (e.g., obj.method() -> (\"obj\", \"method\"))
#[tracing::instrument(skip_all)]
fn extract_instance_method_context(identifier_node: &Node, source: &str) -> Option<(String, String)> {
    let method_name = identifier_node.utf8_text(source.as_bytes()).ok()?.to_string();
    
    // Navigate up to find navigation expression pattern
    let mut current = identifier_node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "navigation_expression" => {
                // Look for pattern: expression . method_name
                let mut object_name = None;
                for child in parent.children(&mut parent.walk()) {
                    match child.kind() {
                        "identifier" | "type_identifier" => {
                            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                                if text != method_name {
                                    object_name = Some(text.to_string());
                                }
                            }
                        }
                        "navigation_suffix" => {
                            // The method name should be in the navigation suffix
                            for suffix_child in child.children(&mut child.walk()) {
                                if matches!(suffix_child.kind(), "identifier") {
                                    if let Ok(text) = suffix_child.utf8_text(source.as_bytes()) {
                                        if text == method_name && object_name.is_some() {
                                            return Some((object_name.unwrap(), method_name));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    
    None
}

/// Resolve variable type by looking for variable declarations
#[tracing::instrument(skip_all)]
fn resolve_variable_type(variable_name: &str, tree: &Tree, source: &str, _context_node: &Node) -> Option<String> {
    use tree_sitter::{Query, QueryCursor};
    
    // Look for variable declarations with the given name
    let query_text = r#"
        (property_declaration
          (variable_declaration
            (identifier) @var_name
            (user_type (type_identifier) @type_name)))
            
        (variable_declaration
          (identifier) @var_name  
          (user_type (type_identifier) @type_name))
    "#;
    
    let language = tree_sitter_kotlin::language();
    let query = Query::new(&language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        let mut found_var_name = None;
        let mut found_type_name = None;
        
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let capture_text = capture.node.utf8_text(source.as_bytes()).ok()?;
            
            match capture_name {
                "var_name" if capture_text == variable_name => {
                    found_var_name = Some(capture_text.to_string());
                }
                "type_name" => {
                    found_type_name = Some(capture_text.to_string());
                }
                _ => {}
            }
        }
        
        if found_var_name.is_some() && found_type_name.is_some() {
            return found_type_name;
        }
    }
    
    None
}

/// Find parent interface or class name for a method
#[tracing::instrument(skip_all)]
fn get_parent_name(tree: &Tree, source: &str, method_name: &str) -> Option<String> {
    use tree_sitter::{Query, QueryCursor};
    
    let query_text = r#"
        ; Interface method
        (interface_declaration
          (type_identifier) @interface_name
          (class_body
            (function_declaration
              (identifier) @method_name)))
              
        ; Class method  
        (class_declaration
          (type_identifier) @class_name
          (class_body
            (function_declaration
              (identifier) @method_name)))
              
        ; Object method
        (object_declaration
          (type_identifier) @object_name
          (class_body
            (function_declaration
              (identifier) @method_name)))
    "#;
    
    let language = tree_sitter_kotlin::language();
    let query = Query::new(&language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        let mut parent_name = None;
        let mut found_method = false;
        
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let capture_text = capture.node.utf8_text(source.as_bytes()).ok()?;
            
            match capture_name {
                "interface_name" | "class_name" | "object_name" => {
                    parent_name = Some(capture_text.to_string());
                }
                "method_name" if capture_text == method_name => {
                    found_method = true;
                }
                _ => {}
            }
        }
        
        if found_method && parent_name.is_some() {
            return parent_name;
        }
    }
    
    None
}

/// Find implementations of a specific interface method
#[tracing::instrument(skip_all)]
async fn find_interface_method_implementations(
    interface_name: &str,
    method_name: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    // First find all implementations of the interface
    let interface_implementations = find_implementations(interface_name, dependency_cache).await?;
    
    let mut method_implementations = Vec::new();
    
    // For each implementation, look for the specific method
    for implementation_location in interface_implementations {
        if let Some(method_location) = find_method_in_class(&implementation_location, method_name).await? {
            method_implementations.push(method_location);
        }
    }
    
    Ok(method_implementations)
}

/// Find a specific method in a class file
#[tracing::instrument(skip_all)]
async fn find_method_in_class(
    class_location: &Location,
    method_name: &str,
) -> Result<Option<Location>> {
    use tokio::fs;
    use tree_sitter::{Query, QueryCursor, StreamingIterator};
    use crate::core::utils::node_to_lsp_location;
    use anyhow::Context;
    
    let file_path = class_location.uri.to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid class file URI"))?;
    
    let source = fs::read_to_string(&file_path).await
        .with_context(|| format!("Failed to read class file: {:?}", file_path))?;
    
    // Parse and search for the method
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_kotlin::language())?;
    
    let tree = parser.parse(&source, None)
        .context("Failed to parse class file")?;
    
    // Use a query to find method declarations with the specific name
    let query_text = r#"
        (function_declaration
            (identifier) @method_name)
    "#;
    
    let query = Query::new(&tree_sitter_kotlin::language(), query_text)?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == method_name {
                    // Found the method, return its location (use the method name node for precise positioning)
                    let uri_string = class_location.uri.to_string();
                    if let Some(location) = node_to_lsp_location(&capture.node, &uri_string) {
                        return Ok(Some(location));
                    }
                }
            }
        }
    }
    
    Ok(None)
}