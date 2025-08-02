use std::{collections::HashSet, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::Node;

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{find_project_root, path_to_file_uri, uri_to_path},
    },
    languages::LanguageSupport,
};

use super::utils::{prepare_symbol_lookup_key, search_definition_in_project};

#[tracing::instrument(skip_all)]
pub fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let current_project = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;

    find_in_project_dependencies(
        source,
        file_uri,
        usage_node,
        &current_project,
        dependency_cache.clone(),
        language_support,
    )
    .or_else(|| {
        fallback_impl(
            source,
            file_uri,
            usage_node,
            dependency_cache,
            language_support,
        )
    })
}

#[tracing::instrument(skip_all)]
fn find_in_project_dependencies(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    current_project: &PathBuf,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let project_meta = dependency_cache.project_metadata.get(current_project)?;

    let symbol_key = prepare_symbol_lookup_key(usage_node, source, file_uri, None)?;
    let (_, fully_qualified_name) = symbol_key;

    // Search in each project that this project depends on
    for dep_project_root in project_meta.inter_project_deps.iter() {
        let dep_symbol_key = (dep_project_root.clone(), fully_qualified_name.clone());

        if let Some(file_location) = dependency_cache.symbol_index.get(&dep_symbol_key) {
            let other_uri = path_to_file_uri(&file_location)?;

            if let Some(location) = search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &other_uri,
                language_support,
            ) {
                return Some(location);
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn fallback_impl(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    // NOTE: Naive implementation, does not consider whether dependency is valid,
    // only checking if the symbol is in the cache.

    debug!("using fallback method");

    let workspace_projects: Vec<PathBuf> = dependency_cache
        .symbol_index
        .iter()
        .map(|entry| entry.key().0.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .into();

    for project_root in workspace_projects.iter() {
        if *project_root == uri_to_path(file_uri).unwrap() {
            // Same-project definitions should be handled by find_in_project function
            continue;
        }

        let symbol_key =
            prepare_symbol_lookup_key(usage_node, source, file_uri, Some(project_root.clone()))?;

        if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
            let other_uri = path_to_file_uri(&file_location)?;

            return search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &other_uri,
                language_support,
            );
        }
    }

    None
}
