use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{
        constants::IS_INDEXING_COMPLETED,
        dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
        jar_utils::get_uri,
        state_manager::get_global,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
        },
    },
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition};

#[tracing::instrument(skip_all)]
pub async fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
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
    .await
}

#[tracing::instrument(skip_all)]
async fn find_project_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    current_project: PathBuf,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    if !get_global(IS_INDEXING_COMPLETED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false) {
        return None;
    }

    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Try to resolve the symbol through imports (including wildcard imports)
    let resolved_symbol = if let Some((_, fully_qualified_name)) =
        prepare_symbol_lookup_key_with_wildcard_support(
            usage_node,
            source,
            file_uri,
            None,
            &dependency_cache,
        ) {
        // Use the full qualified name for external lookup (not just the class name)
        fully_qualified_name
    } else {
        symbol_name.clone()
    };

    // First try current project
    if let Some(source_info) = dependency_cache
        .find_project_external_info(&current_project, &resolved_symbol)
        .await
    {
        return search_external_definition_and_convert(&symbol_name, source_info);
    }

    // Then try projects this project depends on (using project_metadata)
    if let Some(project_metadata) = dependency_cache.project_metadata.get(&current_project) {
        for dependent_project_ref in project_metadata.inter_project_deps.iter() {
            let dependent_project = dependent_project_ref.clone();
            if let Some(source_info) = dependency_cache
                .find_project_external_info(&dependent_project, &resolved_symbol)
                .await
            {
                return search_external_definition_and_convert(&symbol_name, source_info);
            }

            // Also check if the symbol exists directly in the dependency project (not as external dependency)
            if let Some(symbol_path) = dependency_cache
                .find_symbol(&dependent_project, &resolved_symbol)
                .await
            {
                // Convert to external source info format
                let source_info = SourceFileInfo::new(symbol_path, None, None);
                return search_external_definition_and_convert(&symbol_name, source_info);
            }
        }
    }

    // Fallback: try builtin sources (Kotlin standard library, etc.)
    if let Some(builtin_info) = dependency_cache.find_builtin_info(&resolved_symbol) {
        return search_external_definition_and_convert(&symbol_name, builtin_info);
    }

    None
}

fn search_external_definition_and_convert(
    symbol_name: &str,
    source_info: SourceFileInfo,
) -> Option<Location> {
    let tree = source_info.get_tree().ok()?;
    
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