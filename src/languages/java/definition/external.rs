use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::Node;

use crate::{
    core::{
        constants::IS_INDEXING_COMPLETED,
        dependency_cache::{
            source_file_info::SourceFileInfo,
            DependencyCache,
        },
        jar_utils::get_uri,
        state_manager::get_global,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location,
            uri_to_path,
        },
    },
    lsp_warning,
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition};

#[tracing::instrument(skip_all)]
pub fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let current_project = uri_to_path(file_uri).and_then(|path| {
        find_project_root(&path).or_else(|| find_external_dependency_root(&path))
    })?;

    find_project_external(
        source,
        file_uri,
        usage_node,
        current_project,
        dependency_cache.clone(),
    )
}

#[tracing::instrument(skip_all)]
fn find_project_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    current_project: PathBuf,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Try to resolve the symbol through imports (including wildcard imports)
    let resolved_symbol = if let Some((_, fully_qualified_name)) = 
        prepare_symbol_lookup_key_with_wildcard_support(usage_node, source, file_uri, Some(current_project.clone()), &dependency_cache) {
        // Use the full qualified name for external lookup (not just the class name)
        fully_qualified_name
    } else {
        symbol_name.clone()
    };

    let project_key = (current_project.clone(), resolved_symbol.clone());
    debug!("Looking for external dependency with key: {:?}", project_key);
    
    if let Some(source_info) = dependency_cache.project_external_infos.get(&project_key) {
        debug!("Found external dependency for key: {:?}", project_key);
        return search_external_definition_and_convert(&symbol_name, source_info.value().clone());
    } else {
        debug!("External dependency not found for key: {:?}", project_key);
    }
    
    if let Some(source_info) = dependency_cache.builtin_infos.get(&resolved_symbol) {
        return search_external_definition_and_convert(&symbol_name, source_info.value().clone());
    }

    if get_global(IS_INDEXING_COMPLETED).is_none() {
        lsp_warning!("Indexing still in progress...");
    }

    None
}

fn search_external_definition_and_convert(
    symbol_name: &str,
    source_info: SourceFileInfo,
) -> Option<Location> {
    let tree = source_info
        .get_tree()
        .context(format!("failed to get tree for {symbol_name}"))
        .ok()?;

    let content = source_info
        .get_content()
        .context(format!("failed to get content for {symbol_name}"))
        .ok()?;

    let definition_node = search_definition(&tree, &content, symbol_name)?;

    let file_uri = get_uri(&source_info)
        .context(format!("file_uri for {symbol_name} not found"))
        .ok()?;

    node_to_lsp_location(&definition_node, &file_uri)
}