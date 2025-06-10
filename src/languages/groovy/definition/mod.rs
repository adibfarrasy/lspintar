use std::sync::Arc;

use anyhow::{anyhow, Result};
use builtin::find_builtin;
use external::find_external;
use local::find_local;
use project::find_in_project;
use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree};
use utils::set_start_position;
use workspace::find_in_workspace;

use crate::core::dependency_cache::DependencyCache;

mod builtin;
mod external;
mod local;
mod project;
mod workspace;

pub mod utils;

pub fn find_definition_location(
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
) -> Result<Location> {
    find_local(tree, source, file_uri, usage_node)
        .or_else(|| find_in_project(source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_builtin(source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_in_workspace(source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_external(source, file_uri, usage_node, dependency_cache.clone()))
        .and_then(|location| set_start_position(source, usage_node, &location.uri.to_string()))
        .ok_or_else(|| anyhow!("Definition not found"))
}
