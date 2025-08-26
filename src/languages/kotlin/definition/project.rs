use std::sync::Arc;

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{dependency_cache::DependencyCache, utils::path_to_file_uri},
    languages::LanguageSupport,
};

use super::utils::{
    extract_package_from_source, prepare_symbol_lookup_key_with_wildcard_support,
    resolve_symbol_with_imports, search_definition_in_project,
};

#[tracing::instrument(skip_all)]
pub async fn find_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_key = prepare_symbol_lookup_key_with_wildcard_support(
        usage_node,
        source,
        file_uri,
        None,
        &dependency_cache,
    );

    let (project_root, fqn) = if let Some(key) = symbol_key {
        key
    } else {
        let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
        let project_root = crate::core::utils::uri_to_path(file_uri)
            .and_then(|path| crate::core::utils::find_project_root(&path))?;

        // Try to resolve FQN using imports first, then fallback to current package
        let fqn = if let Some(resolved_fqn) =
            resolve_symbol_with_imports(&symbol_name, source, &dependency_cache)
        {
            resolved_fqn
        } else if let Some(package) = extract_package_from_source(source) {
            if !package.is_empty() {
                format!("{}.{}", package, symbol_name)
            } else {
                symbol_name.clone()
            }
        } else {
            symbol_name.clone()
        };

        (project_root, fqn)
    };

    let file_location = dependency_cache.find_symbol(&project_root, &fqn).await?;

    let other_uri = path_to_file_uri(&file_location)?;

    if file_uri == &other_uri {
        // Local definitions should be handled by find_local function
        return None;
    }

    search_definition_in_project(file_uri, source, usage_node, &other_uri, language_support)
}
