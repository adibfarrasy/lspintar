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

fn handle_method_call_implementation(
    _tree: &Tree,
    _source: &str,
    _position: Position,
    _dependency_cache: Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    
    Ok(vec![])
}

async fn find_method_implementations(
    _tree: &Tree,
    _source: &str,
    _method_name: &str,
    _dependency_cache: &Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    
    Ok(vec![])
}