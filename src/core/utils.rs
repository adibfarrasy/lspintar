use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use tower_lsp::lsp_types::Url;
use tree_sitter::{Parser, Tree};

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
