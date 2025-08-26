use anyhow::Result;
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::task;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::Tree;

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::path_to_file_uri,
    },
    languages::LanguageSupport,
};

use super::utils::find_identifier_at_position;

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
            handle_method_call_implementation(tree, source, position, dependency_cache, language_support)
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
        if let Some(mut project_locations) = task.await? {
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

fn handle_method_call_implementation(
    _tree: &Tree,
    _source: &str,
    _position: Position,
    _dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Result<Vec<Location>> {
    // TODO: Implement method call -> implementation mapping
    // This would involve:
    // 1. Finding the method declaration that this call refers to
    // 2. If it's an interface/abstract method, finding concrete implementations
    Ok(vec![])
}

async fn find_method_implementations(
    _tree: &Tree,
    _source: &str,
    _method_name: &str,
    _dependency_cache: &Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    // TODO: Implement method implementation finding
    // This would involve:
    // 1. Determining if the method is abstract/interface method
    // 2. Finding classes that implement the interface or extend the abstract class
    // 3. Looking for method implementations in those classes
    Ok(vec![])
}