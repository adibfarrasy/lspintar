use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tracing::debug;
use tree_sitter::{Node, Parser, Tree};

use super::constants::PROJECT_ROOT_MARKER;

#[tracing::instrument(skip_all)]
pub fn path_to_file_uri(file_path: &PathBuf) -> Option<String> {
    let url = Url::from_file_path(file_path)
        .inspect_err(|_| debug!("Cannot convert file_path {:#?} to Url", file_path))
        .ok()?;

    Some(url.to_string())
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri)
        .inspect_err(|_| debug!("Cannot convert uri {uri} to Url"))
        .ok()?;
    url.to_file_path().ok()
}

pub fn find_project_root(file_path: &Path) -> Option<PathBuf> {
    let mut current = file_path.parent()?;

    loop {
        if PROJECT_ROOT_MARKER
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return Some(current.to_path_buf());
        }

        current = current.parent()?;
    }
}

pub fn create_parser_for_language(language: &str) -> Option<Parser> {
    let mut parser = Parser::new();

    match language {
        "groovy" => {
            parser
                .set_language(&tree_sitter_groovy::language())
                .inspect_err(|e| debug!("Cannot set groovy parser: {e}"))
                .ok()?;
        }
        "java" => {
            parser
                .set_language(&tree_sitter_java::LANGUAGE.into())
                .inspect_err(|e| debug!("Cannot set java parser: {e}"))
                .ok()?;
        }
        _ => return None,
    };

    Some(parser)
}

pub fn detect_language_from_path(file_path: &PathBuf) -> Option<&'static str> {
    match file_path.extension()?.to_str()? {
        "java" => Some("java"),
        "groovy" | "gradle" => Some("groovy"),
        "kt" | "kts" => Some("kotlin"),
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
pub fn uri_to_tree(uri: &str) -> Option<Tree> {
    let file_path = uri_to_path(uri)?;

    let file_content = read_to_string(&file_path)
        .inspect_err(|_| debug!("Cannot get file content from file_path {:#?}", file_path))
        .ok()?;

    let language = detect_language_from_path(&file_path)?;

    let mut parser = create_parser_for_language(language)?;

    parser.parse(&file_content, None)
}

pub fn node_contains_position(node: &Node, position: Position) -> bool {
    let start = node.start_position();
    let end = node.end_position();

    let pos_line = position.line as usize;
    let pos_char = position.character as usize;

    (start.row < pos_line || (start.row == pos_line && start.column <= pos_char))
        && (pos_line < end.row || (pos_line == end.row && pos_char <= end.column))
}

#[tracing::instrument(skip_all)]
pub fn node_to_lsp_location(node: &Node, file_uri: &str) -> Option<Location> {
    let start_pos = node.start_position();
    let end_pos = node.end_position();

    let range = Range {
        start: Position {
            line: start_pos.row as u32,
            character: start_pos.column as u32,
        },
        end: Position {
            line: end_pos.row as u32,
            character: end_pos.column as u32,
        },
    };

    let uri = Url::parse(file_uri)
        .inspect_err(|e| debug!("Failed to parse URI: {e}"))
        .ok()?;
    Some(Location { uri, range })
}

// Only get the closest node to root
#[tracing::instrument(skip_all)]
pub fn location_to_node<'a>(location: &Location, tree: &'a Tree) -> Option<Node<'a>> {
    let position = location.range.start;
    find_node_at_position(tree, position)
}

fn find_node_at_position<'a>(tree: &'a Tree, position: Position) -> Option<Node<'a>> {
    let mut current = tree.root_node();

    loop {
        let mut found_child = None;
        let mut cursor = current.walk();

        for child in current.children(&mut cursor) {
            if node_contains_position(&child, position) {
                found_child = Some(child);
                break;
            }
        }

        match found_child {
            Some(child) => {
                current = child;
            }
            None => {
                break;
            }
        }
    }

    if node_contains_position(&current, position) {
        Some(current)
    } else {
        None
    }
}

pub fn is_project_root(current: &PathBuf) -> bool {
    PROJECT_ROOT_MARKER
        .iter()
        .any(|marker| current.join(marker).exists())
}
