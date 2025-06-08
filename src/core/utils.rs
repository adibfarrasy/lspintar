use std::path::{Path, PathBuf};

use tower_lsp::lsp_types::Url;
use tree_sitter::Parser;

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
        if current.join("pom.xml").exists() {
            return Some(current.to_path_buf());
        }

        if current.join("build.gradle").exists() || current.join("build.gradle.kts").exists() {
            return Some(current.to_path_buf());
        }

        if current.join(".git").exists() {
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
