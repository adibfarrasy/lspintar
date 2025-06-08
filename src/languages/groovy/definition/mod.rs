use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use builtin::find_builtin;
use external::find_external;
use local::find_local;
use project::find_in_project;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};
use workspace::find_in_workspace;

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{
            create_parser_for_language, detect_language_from_path, find_project_root,
            path_to_file_uri, uri_to_path,
        },
    },
    languages::groovy::symbols::SymbolType,
};

mod builtin;
mod external;
mod local;
mod project;
mod utils;
mod workspace;

pub use utils::find_identifier_at_position;

pub fn find_definition_location(
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
) -> Result<Location> {
    find_local(tree, source, file_uri, usage_node)
        .or_else(|| find_in_project(tree, source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_builtin(tree, source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_in_workspace(tree, source, file_uri, usage_node, dependency_cache.clone()))
        .or_else(|| find_external(tree, source, file_uri, usage_node, dependency_cache.clone()))
        .ok_or_else(|| anyhow!("Definition not found"))
}
