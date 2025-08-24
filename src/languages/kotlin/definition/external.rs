use std::sync::Arc;
use tower_lsp::lsp_types::Location;
use tree_sitter::Node;
use crate::core::dependency_cache::DependencyCache;

pub async fn find_external(
    _source: &str,
    _file_uri: &str,
    _usage_node: &Node<'_>,
    _dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // TODO: Implement Kotlin external definition search
    // This would search in external dependencies (JAR files, etc.)
    None
}