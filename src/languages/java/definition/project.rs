use std::sync::Arc;

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{dependency_cache::DependencyCache, utils::path_to_file_uri},
    languages::LanguageSupport,
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition_in_project};

#[tracing::instrument(skip_all)]
pub fn find_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_key = prepare_symbol_lookup_key_with_wildcard_support(usage_node, source, file_uri, None, &dependency_cache)?;

    let file_location = dependency_cache.symbol_index.get(&symbol_key)?;

    let other_uri = path_to_file_uri(&file_location.clone())?;

    if file_uri == &other_uri {
        // Local definitions should be handled by find_local function
        return None;
    }

    search_definition_in_project(file_uri, source, usage_node, &other_uri, language_support)
}