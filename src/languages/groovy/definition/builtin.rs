use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use log::debug;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::dependency_cache::DependencyCache;

pub fn find_builtin(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    debug!("find_builtin scope");

    // TODO: implement
    None
}
