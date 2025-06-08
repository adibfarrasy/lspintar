use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

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

pub fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // TODO: implement
    None
}
