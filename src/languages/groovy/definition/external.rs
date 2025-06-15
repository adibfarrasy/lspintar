use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use log::debug;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::dependency_cache::DependencyCache;

pub fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // FIXME: implement
    // Step 1: Read the external file

    // Step 2: Determine language and create parser

    // Step 3: Parse the external file

    // Step 4: Search for the symbol definition in the external tree

    // Step 5: Convert file path to URI

    // Step 6: Convert node to Location

    None
}
