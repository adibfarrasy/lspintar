use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::dependency_cache::DependencyCache;

pub fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // TODO: implement
    None
}
