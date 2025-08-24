use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::Node;

use crate::{
    core::{
        constants::IS_INDEXING_COMPLETED,
        dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
        jar_utils::get_uri,
        state_manager::get_global,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
        },
    },
    lsp_warning,
};

use super::method_resolution::find_method_with_signature;
use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition};

#[tracing::instrument(skip_all)]
pub async fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    debug!(
        "find_external: looking for external symbol '{}'",
        symbol_name
    );

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
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    debug!(
        "find_project_external: looking for external symbol '{}'",
        symbol_name
    );

    // Try to resolve the symbol through imports (including wildcard imports)
    debug!(
        "find_project_external: attempting symbol resolution for '{}' in file '{}'",
        symbol_name, file_uri
    );
    let resolved_symbol = if let Some((_, fully_qualified_name)) =
        prepare_symbol_lookup_key_with_wildcard_support(
            usage_node,
            source,
            file_uri,
            None,
            &dependency_cache,
        ) {
        debug!(
            "find_project_external: symbol resolved to '{}'",
            fully_qualified_name
        );
        // Use the full qualified name for external lookup (not just the class name)
        fully_qualified_name
    } else {
        debug!(
            "find_project_external: symbol resolution failed for '{}', using original name '{}'",
            symbol_name, symbol_name
        );
        symbol_name.clone()
    };

    // First try current project
    if let Some(external_info) = dependency_cache
        .find_project_external_info(&current_project, &resolved_symbol)
        .await
    {
        debug!("find_project_external: found external info in current project");
        return search_external_definition_and_convert(&symbol_name, external_info);
    }

    // Then try projects this project depends on (using project_metadata)
    if let Some(project_metadata) = dependency_cache.project_metadata.get(&current_project) {
        debug!(
            "find_project_external: checking {} inter-project dependencies",
            project_metadata.inter_project_deps.len()
        );
        for dependent_project_ref in project_metadata.inter_project_deps.iter() {
            let dependent_project = dependent_project_ref.clone();
            debug!(
                "find_project_external: checking dependency project '{:?}'",
                dependent_project
            );
            if let Some(external_info) = dependency_cache
                .find_project_external_info(&dependent_project, &resolved_symbol)
                .await
            {
                debug!(
                    "find_project_external: found external info in dependency project '{:?}'",
                    dependent_project
                );
                return search_external_definition_and_convert(&symbol_name, external_info);
            }

            // Also check if the symbol exists directly in the dependency project (not as external dependency)
            debug!(
                "find_project_external: searching for symbol '{}' in dependency project '{:?}'",
                resolved_symbol, dependent_project
            );
            if let Some(symbol_path) = dependency_cache
                .find_symbol(&dependent_project, &resolved_symbol)
                .await
            {
                debug!("find_project_external: found symbol in dependency project '{:?}' at path '{:?}'", dependent_project, symbol_path);
                // Convert to external source info format
                let external_info = SourceFileInfo::new(symbol_path, None, None);
                return search_external_definition_and_convert(&symbol_name, external_info);
            } else {
                debug!(
                    "find_project_external: symbol '{}' not found in dependency project '{:?}'",
                    resolved_symbol, dependent_project
                );
            }
        }
    } else {
        debug!(
            "find_project_external: no project metadata found for '{:?}'",
            current_project
        );

        // Fallback: try all other projects in the cache (as before)
        let mut checked_projects = std::collections::HashSet::new();
        checked_projects.insert(current_project.clone());

        for entry in dependency_cache.project_external_infos.iter() {
            let (project_root, _) = entry.key();
            if !checked_projects.contains(project_root) {
                checked_projects.insert(project_root.clone());
                debug!(
                    "find_project_external: checking project external infos in '{:?}' (fallback)",
                    project_root
                );
                if let Some(external_info) = dependency_cache
                    .find_project_external_info(project_root, &resolved_symbol)
                    .await
                {
                    debug!(
                        "find_project_external: found external info in project '{:?}' (fallback)",
                        project_root
                    );
                    return search_external_definition_and_convert(&symbol_name, external_info);
                }
            }
        }
    }

    if let Some(external_info) = dependency_cache.find_builtin_info(&resolved_symbol) {
        return search_external_definition_and_convert(&symbol_name, external_info);
    }

    if get_global(IS_INDEXING_COMPLETED).is_none() {
        lsp_warning!("Indexing still in progress...");
    }

    None
}

#[tracing::instrument(skip_all)]
fn search_external_definition_and_convert(
    symbol_name: &str,
    external_info: SourceFileInfo,
) -> Option<Location> {
    let tree = external_info
        .get_tree()
        .context(format!("failed to get tree for {symbol_name}"))
        .ok()?;

    let content = external_info
        .get_content()
        .context(format!("failed to get content for {symbol_name}"))
        .ok()?;

    let definition_node = search_definition(&tree, &content, symbol_name, SymbolType::Type)
        .context(format!("definition for {symbol_name} not found"))
        .ok()?;

    let file_uri = get_uri(&external_info.clone())
        .context(format!("file_uri for {symbol_name} not found"))
        .ok()?;

    node_to_lsp_location(&definition_node, &file_uri)
}

/// Enhanced external method resolution that handles method calls in external dependencies
#[tracing::instrument(skip_all)]
fn search_external_method_definition_and_convert(
    method_name: &str,
    call_signature: Option<super::method_resolution::CallSignature>,
    external_info: SourceFileInfo,
) -> Option<Location> {
    let tree = external_info
        .get_tree()
        .context(format!("failed to get tree for method {method_name}"))
        .ok()?;

    let content = external_info
        .get_content()
        .context(format!("failed to get content for method {method_name}"))
        .ok()?;

    let definition_node = if let Some(call_sig) = call_signature {
        // Use signature-based method matching for external methods
        find_method_with_signature(&tree, &content, method_name, &call_sig)
    } else {
        // Fallback to simple method search
        search_definition(&tree, &content, method_name, SymbolType::MethodCall)
    }
    .context(format!("method definition for {method_name} not found"))
    .ok()?;

    let file_uri = get_uri(&external_info.clone())
        .context(format!("file_uri for method {method_name} not found"))
        .ok()?;

    node_to_lsp_location(&definition_node, &file_uri)
}
