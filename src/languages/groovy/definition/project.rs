use std::sync::Arc;

use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::{
    core::{dependency_cache::DependencyCache, utils::path_to_file_uri},
    languages::LanguageSupport,
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition_in_project};

#[tracing::instrument(skip_all)]
pub async fn find_in_project(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    // First try wildcard resolution
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
        // Fallback: try direct symbol lookup (for symbols that don't need import resolution)
        let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
        let project_root = crate::core::utils::uri_to_path(file_uri)
            .and_then(|path| crate::core::utils::find_project_root(&path))?;


        // Try to construct FQN using the current package
        let fqn = if let Some(package) = extract_package_from_source(source) {
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

/// Extract package name from Groovy source code
fn extract_package_from_source(source: &str) -> Option<String> {
    let query_text = r#"
        (package_declaration
          (scoped_identifier) @package_name)
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
                return;
            }

            for capture in query_match.captures {
                if let Ok(package_name) = capture.node.utf8_text(source.as_bytes()) {
                    result = Some(package_name.to_string());
                    return;
                }
            }
        });

    result
}
