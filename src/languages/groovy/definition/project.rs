use std::{fs::read_to_string, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::core::{
    dependency_cache::DependencyCache,
    utils::{find_project_root, path_to_file_uri, uri_to_path, uri_to_tree},
};

use super::utils::{determine_symbol_type_from_context, node_to_lsp_location, search_definition};

pub fn find_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_key = prepare_symbol_lookup_key(usage_node, source, file_uri)?;

    let file_location = dependency_cache.symbol_index.get(&symbol_key)?;

    let other_uri = path_to_file_uri(&file_location.clone())?;

    if file_uri == &other_uri {
        // Local definitions should be handled by find_local function
        return None;
    }

    search_definition_in_project(file_uri, source, usage_node, &other_uri)
}

fn prepare_symbol_lookup_key(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = find_project_root(&current_file_path)?;

    resolve_through_imports(&symbol_name, source, &project_root)
}

fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (import_declaration
          (fully_qualified_name) @import_name) 

        (import_declaration
          (wildcard_import) @import_name) 
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
                return; // Already found a match
            }

            for capture in query_match.captures {
                if let Ok(import_text) = capture.node.utf8_text(source.as_bytes()) {
                    if import_text.ends_with(&format!(".{}", symbol_name)) {
                        result = Some((project_root.clone(), import_text.to_string()));
                        return;
                    }

                    // Check for wildcard import - do actual classpath lookup
                    if import_text.ends_with(".*") {
                        todo!()
                    }
                };
            }
        });

    result
}

fn search_definition_in_project(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &Node,
    other_file_uri: &str,
) -> Option<Location> {
    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;
    let symbol_type =
        determine_symbol_type_from_context(&current_tree, usage_node, current_source).ok()?;

    let other_tree = uri_to_tree(other_file_uri)?;
    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    let definition_node = search_definition(&other_tree, &other_source, symbol_name, symbol_type)?;

    return node_to_lsp_location(&definition_node, &other_file_uri);
}
