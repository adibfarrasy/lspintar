use std::{collections::HashSet, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{path_to_file_uri, uri_to_path},
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
    // NOTE: Naive implementation, does not consider whether dependency is
    // valid, only checking if the symbol is in the cache.
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
