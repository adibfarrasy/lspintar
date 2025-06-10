use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use tower_lsp::lsp_types::{Hover, Location, Position, Range, Url};
use tree_sitter::{Node, Parser, Tree};

use super::constants::PROJECT_ROOT_MARKER;

pub fn path_to_file_uri(file_path: &PathBuf) -> Option<String> {
    let url = Url::from_file_path(file_path).ok()?;
    Some(url.to_string())
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri).ok()?;
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

    let tree_sitter_language = match language {
        "groovy" => tree_sitter_groovy::language(),
        // "java" => tree_sitter_java::language(),
        _ => return None,
    };

    parser.set_language(&tree_sitter_language).ok()?;
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

pub fn uri_to_tree(uri: &str) -> Option<Tree> {
    let file_path = uri_to_path(uri)?;

    let file_content = read_to_string(&file_path).ok()?;

    let language = detect_language_from_path(&file_path)?;

    let mut parser = create_parser_for_language(language)?;

    parser.parse(&file_content, None)
}

pub fn location_to_hover(location: Location) -> Hover {
    todo!()
}

pub fn node_contains_position(node: &Node, position: Position) -> bool {
    let start = node.start_position();
    let end = node.end_position();

    let pos_line = position.line as usize;
    let pos_char = position.character as usize;

    (start.row < pos_line || (start.row == pos_line && start.column <= pos_char))
        && (pos_line < end.row || (pos_line == end.row && pos_char <= end.column))
}

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

    let uri = Url::parse(file_uri).ok()?;
    Some(Location { uri, range })
}
