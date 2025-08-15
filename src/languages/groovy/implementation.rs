use anyhow::{Context, Result};
use log::debug;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio::{fs, task};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Query, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{node_to_lsp_location, path_to_file_uri},
    },
    languages::LanguageSupport,
};

use super::utils::find_identifier_at_position;

static IMPLEMENTATION_WITH_METHOD_QUERY: OnceLock<Option<Query>> = OnceLock::new();

static INTERFACE_DECLARATION_QUERY: OnceLock<Option<Query>> = OnceLock::new();

pub fn handle(
    tree: &Tree,
    source: &str,
    position: Position,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Result<Vec<Location>> {
    let identifier_node = find_identifier_at_position(tree, source, position)?;
    let symbol_name = identifier_node.utf8_text(source.as_bytes())?;
    let symbol_type =
        language_support.determine_symbol_type_from_context(tree, &identifier_node, source)?;

    match symbol_type {
        SymbolType::InterfaceDeclaration | SymbolType::ClassDeclaration | SymbolType::Type => {
            futures::executor::block_on(find_implementations(symbol_name, &dependency_cache))
        }
        SymbolType::MethodCall => {
            handle_method_call_implementation(tree, source, position, dependency_cache, language_support)
        }
        SymbolType::MethodDeclaration => futures::executor::block_on(async {
            // TODO: currently only handle interfaces.
            // implement abstract class handling next.

            let parent_name =
                get_parent_name(tree, source, symbol_name).context("Failed to get parent name")?;

            let locations = find_implementations(&parent_name, &dependency_cache).await?;

            find_method_implementations(symbol_name, locations).await
        }),
        _ => Ok(vec![]),
    }
}

#[tracing::instrument(skip_all)]
async fn find_implementations(
    interface_name: &str,
    dependency_cache: &DependencyCache,
) -> Result<Vec<Location>> {
    let project_roots: std::collections::HashSet<PathBuf> = dependency_cache
        .inheritance_index
        .iter()
        .map(|entry| entry.key().0.clone())
        .collect();

    let tasks: Vec<_> = project_roots
        .into_iter()
        .map(|project_root| {
            let interface_name = interface_name.to_string();
            let inheritance_index = dependency_cache.inheritance_index.clone();

            task::spawn(async move {
                inheritance_index
                    .get(&(project_root, interface_name))
                    .map(|file_paths| file_paths.value().clone())
            })
        })
        .collect();

    let results = futures::future::join_all(tasks).await;

    let mut all_locations = Vec::new();
    for result in results {
        if let Ok(Some(index_value)) = result {
            for (file_path, line, col) in index_value {
                if let Some(file_uri) = path_to_file_uri(&file_path) {
                    let uri = Url::parse(&file_uri)
                        .inspect_err(|e| debug!("Failed to parse URI: {e}"))?;
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
fn get_parent_name(tree: &Tree, source: &str, symbol_name: &str) -> Option<String> {
    let mut cursor = tree_sitter::QueryCursor::new();

    let query = get_interface_declaration_query()
        .as_ref()
        .context("Failed to get query")
        .ok()?;

    let mut interface_name = None;
    let mut found = false;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found {
                return;
            }

            for capture in query_match.captures {
                match capture.index {
                    0 => {
                        if let Ok(name) = capture.node.utf8_text(source.as_bytes()) {
                            interface_name = Some(name.to_string());
                        }
                    }
                    1 => {
                        if let Ok(method_name) = capture.node.utf8_text(source.as_bytes()) {
                            if method_name == symbol_name {
                                found = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

    interface_name
}

#[tracing::instrument(skip_all)]
async fn find_method_implementations(
    symbol_name: &str,
    locations: Vec<Location>,
) -> Result<Vec<Location>> {
    // TODO: currently using naive implementation
    // should handle method overloading next.

    let mut results = Vec::new();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_groovy::language())?;

    let query = get_implementation_with_method_query()
        .as_ref()
        .context("Failed to get query")?;

    for location in locations {
        let content = fs::read_to_string(&location.uri.path()).await?;
        let tree = parser.parse(&content, None).context("failed to parse")?;
        let mut cursor = tree_sitter::QueryCursor::new();

        cursor
            .matches(&query, tree.root_node(), content.as_bytes())
            .for_each(|query_match| {
                for capture in query_match.captures {
                    if capture.index == 2 {
                        let method_name = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        if method_name == symbol_name {
                            if let Some(loc) =
                                node_to_lsp_location(&capture.node, &location.uri.to_string())
                            {
                                results.push(loc);
                            }
                        }
                    }
                }
            })
    }

    Ok(results)
}

#[tracing::instrument(skip_all)]
fn get_implementation_with_method_query() -> &'static Option<Query> {
    IMPLEMENTATION_WITH_METHOD_QUERY.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        let text = r#"(class_declaration 
                name: (identifier) @class_name
                interfaces: (super_interfaces 
                    (type_list (type_identifier) @interface_name))
                body: (class_body
                    (method_declaration (identifier) @method_name))
                )"#;

        Query::new(&language, text)
            .context("failed to parse query")
            .ok()
    })
}

#[tracing::instrument(skip_all)]
fn get_interface_declaration_query() -> &'static Option<Query> {
    INTERFACE_DECLARATION_QUERY.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        let text = r#"
        (interface_declaration 
            name: (identifier) @interface_name
            body: (interface_body 
                (method_declaration name: (identifier) @method_name)))
        "#;

        Query::new(&language, text)
            .inspect_err(|error| debug!("[get_interface_declaration_query] {error}"))
            .ok()
    })
}

/// Handle go-to-implementation for method calls like someService.method()
fn handle_method_call_implementation(
    tree: &Tree,
    source: &str,
    position: Position,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Result<Vec<Location>> {
    let identifier_node = find_identifier_at_position(tree, source, position)?;
    
    // Step 1: Check if this is an instance method call and get the variable/method info
    let instance_context = super::definition::utils::extract_instance_method_context(&identifier_node, source);
    
    if let Some((variable_name, method_name)) = instance_context {
        // Step 2: Resolve the variable type to get the interface/class name  
        let variable_type = super::definition::utils::resolve_variable_type(&variable_name, tree, source, &identifier_node);
        
        if let Some(class_name) = variable_type {
            // Step 3: Find implementations of this class/interface and look for the method
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

/// Find a node at a specific position in the tree
fn find_node_at_position(tree: &Tree, position: Position) -> Option<tree_sitter::Node> {
    let point = tree_sitter::Point {
        row: position.line as usize,
        column: position.character as usize,
    };
    
    tree.root_node().descendant_for_point_range(point, point)
}

/// Check if a node is contained within an interface declaration
fn find_containing_interface(mut node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    while let Some(parent) = node.parent() {
        if parent.kind() == "interface_declaration" {
            return Some(parent);
        }
        node = parent;
    }
    None
}

/// Find implementations of a specific interface method
async fn find_interface_method_implementations(
    interface_name: &str,
    method_name: &str,
    dependency_cache: &DependencyCache,
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
    parser.set_language(&tree_sitter_groovy::language())?;
    
    let tree = parser.parse(&source, None)
        .context("Failed to parse class file")?;
    
    // Use a query to find method declarations with the specific name
    let query_text = r#"
        (method_declaration
            name: (identifier) @method_name)
    "#;
    
    let query = Query::new(&tree_sitter_groovy::language(), query_text)?;
    let mut cursor = tree_sitter::QueryCursor::new();
    
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
