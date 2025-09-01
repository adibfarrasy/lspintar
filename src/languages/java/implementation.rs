use anyhow::{Context, Result};
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::{fs, task};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{node_to_lsp_location, path_to_file_uri},
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
    
    // If no in-memory data, get project roots from symbol index (fallback)
    if project_roots.is_empty() {
        project_roots = dependency_cache
            .symbol_index
            .iter()
            .map(|entry| entry.key().0.clone())
            .collect();
    }

    let tasks: Vec<_> = project_roots
        .into_iter()
        .map(|project_root| {
            let interface_name = interface_name.to_string();
            let dependency_cache = dependency_cache.clone();

            task::spawn(async move {
                dependency_cache
                    .find_inheritance_implementations(&project_root, &interface_name)
                    .await
            })
        })
        .collect();

    let results = futures::future::join_all(tasks).await;

    let mut all_locations = Vec::new();
    for result in results {
        if let Ok(Some(index_value)) = result {
            for (file_path, line, col) in index_value {
                if let Some(file_uri) = path_to_file_uri(&file_path) {
                    let uri = Url::parse(&file_uri).map_err(anyhow::Error::from)?;
                    let location = Location {
                        uri,
                        range: Range {
                            start: Position {
                                line: line as u32,
                                character: col as u32,
                            },
                            end: Position {
                                line: line as u32,
                                character: col as u32,
                            },
                        },
                    };
                    all_locations.push(location);
                }
            }
        }
    }

    Ok(all_locations)
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

/// Find parent interface or class name for a method
#[tracing::instrument(skip_all)]
fn get_parent_name(tree: &Tree, source: &str, method_name: &str) -> Option<String> {
    let query_text = r#"
        ; Interface method
        (interface_declaration
          (identifier) @interface_name
          (interface_body
            (method_declaration
              (identifier) @method_name)))
              
        ; Class method  
        (class_declaration
          (identifier) @class_name
          (class_body
            (method_declaration
              (identifier) @method_name)))
    "#;
    
    let language = tree_sitter_java::LANGUAGE.into();
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
                "interface_name" | "class_name" => {
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

/// Find a specific method in a class file
#[tracing::instrument(skip_all)]
async fn find_method_in_class(
    class_location: &Location,
    method_name: &str,
) -> Result<Option<Location>> {
    let file_path = class_location.uri.to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid class file URI"))?;
    
    let source = fs::read_to_string(&file_path).await
        .with_context(|| format!("Failed to read class file: {:?}", file_path))?;
    
    // Parse and search for the method
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_java::LANGUAGE.into())?;
    
    let tree = parser.parse(&source, None)
        .context("Failed to parse class file")?;
    
    // Use a query to find method declarations with the specific name
    let query_text = r#"
        (method_declaration
            (identifier) @method_name)
    "#;
    
    let query = Query::new(&tree_sitter_java::LANGUAGE.into(), query_text)?;
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

/// Extract instance method context from an identifier node (e.g., obj.method() -> ("obj", "method"))
#[tracing::instrument(skip_all)]
fn extract_instance_method_context(identifier_node: &tree_sitter::Node, source: &str) -> Option<(String, String)> {
    let method_name = identifier_node.utf8_text(source.as_bytes()).ok()?.to_string();
    
    // Navigate up to find method_invocation pattern
    let mut current = identifier_node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "method_invocation" => {
                // Look for pattern: object . method_name
                let mut object_name = None;
                for child in parent.children(&mut parent.walk()) {
                    match child.kind() {
                        "identifier" => {
                            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                                if text != method_name {
                                    object_name = Some(text.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                
                if let Some(obj_name) = object_name {
                    return Some((obj_name, method_name));
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
fn resolve_variable_type(variable_name: &str, tree: &Tree, source: &str, _context_node: &tree_sitter::Node) -> Option<String> {
    // Look for variable declarations with the given name
    let query_text = r#"
        (local_variable_declaration
          type: (type_identifier) @type_name
          declarator: (variable_declarator
            name: (identifier) @var_name))
            
        (field_declaration
          type: (type_identifier) @type_name
          declarator: (variable_declarator
            name: (identifier) @var_name))
    "#;
    
    let language = tree_sitter_java::LANGUAGE.into();
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

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_java_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        let language = tree_sitter_java::LANGUAGE;
        parser.set_language(&language.into()).ok()?;
        Some(parser)
    }

    #[test]
    fn test_get_parent_name_interface_method() {
        let mut parser = create_java_parser().unwrap();
        let source = r#"
interface TestInterface {
    void testMethod();
    String anotherMethod();
}
        "#;

        let tree = parser.parse(source, None).unwrap();
        let result = get_parent_name(&tree, source, "testMethod");
        assert_eq!(result, Some("TestInterface".to_string()));
    }

    #[test]
    fn test_get_parent_name_class_method() {
        let mut parser = create_java_parser().unwrap();
        let source = r#"
class TestClass {
    public void testMethod() {
        // implementation
    }
    
    private String helper() {
        return "test";
    }
}
        "#;

        let tree = parser.parse(source, None).unwrap();
        let result = get_parent_name(&tree, source, "testMethod");
        assert_eq!(result, Some("TestClass".to_string()));
    }

    #[test]
    fn test_get_parent_name_method_not_found() {
        let mut parser = create_java_parser().unwrap();
        let source = r#"
class TestClass {
    public void otherMethod() {
        // implementation
    }
}
        "#;

        let tree = parser.parse(source, None).unwrap();
        let result = get_parent_name(&tree, source, "nonExistentMethod");
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_variable_type() {
        let mut parser = create_java_parser().unwrap();
        let source = r#"
public class TestClass {
    private MyInterface field;
    
    public void someMethod() {
        String localVar = "test";
        MyInterface obj = getInterface();
        obj.call();
    }
}
        "#;

        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Test local variable resolution
        let local_var_type = resolve_variable_type("localVar", &tree, source, &root);
        assert_eq!(local_var_type, Some("String".to_string()));

        // Test interface variable resolution
        let obj_type = resolve_variable_type("obj", &tree, source, &root);
        assert_eq!(obj_type, Some("MyInterface".to_string()));

        // Test field variable resolution
        let field_type = resolve_variable_type("field", &tree, source, &root);
        assert_eq!(field_type, Some("MyInterface".to_string()));

        // Test non-existent variable
        let nonexistent_type = resolve_variable_type("nonExistent", &tree, source, &root);
        assert_eq!(nonexistent_type, None);
    }
}