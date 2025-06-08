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

pub fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    // TODO: implement
    None
}

// fn search_in_external_file_for_location(
//     file_path: &PathBuf,
//     usage_node: &Node,
// ) -> Option<Location> {
//     // Step 1: Read the external file
//     let content = std::fs::read_to_string(file_path).ok()?;
//
//     // Step 2: Determine language and create parser
//     let language = detect_language_from_path(file_path)?;
//     let mut parser = create_parser_for_language(language)?;
//
//     // Step 3: Parse the external file
//     let tree = parser.parse(&content, None)?;
//
//     // Step 4: Search for the symbol definition in the external tree
//     let definition_node = search_local_definitions(&tree, &content, usage_node)?;
//
//     // Step 5: Convert file path to URI
//     let file_uri = path_to_file_uri(file_path)?;
//
//     // Step 6: Convert node to Location
//     node_to_lsp_location(&definition_node, &file_uri)
// }
