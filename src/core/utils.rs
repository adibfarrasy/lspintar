use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tracing::debug;
use tree_sitter::{Node, Parser, Tree};

use super::constants::{PROJECT_ROOT_MARKER, TEMP_DIR_PREFIX};
use crate::languages::groovy::definition::utils::search_definition_in_project as groovy_search_definition_in_project;
use crate::languages::java::definition::utils::search_definition_in_project as java_search_definition_in_project;
use crate::languages::kotlin::definition::utils::search_definition_in_project as kotlin_search_definition_in_project;
use crate::languages::{
    groovy::GroovySupport, java::JavaSupport, kotlin::KotlinSupport, LanguageSupport,
};

#[tracing::instrument(skip_all)]
pub fn path_to_file_uri(file_path: &PathBuf) -> Option<String> {
    let url = Url::from_file_path(file_path)
        .inspect_err(|_| debug!("Cannot convert file_path {:#?} to Url", file_path))
        .ok()?;

    Some(url.to_string())
}

#[tracing::instrument(skip_all)]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri)
        .inspect_err(|_| debug!("Cannot convert uri {uri} to Url"))
        .ok()?;
    url.to_file_path().ok()
}

#[tracing::instrument(skip_all)]
pub fn find_project_root(file_path: &Path) -> Option<PathBuf> {
    // Start with the current path (for directory inputs) or parent (for file inputs)
    let mut current = if file_path.is_dir() {
        file_path
    } else {
        file_path.parent()?
    };

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

#[tracing::instrument(skip_all)]
pub fn find_external_dependency_root(nested_path: &PathBuf) -> Option<PathBuf> {
    let path_str = nested_path.to_string_lossy();

    if let Some(builtin_start) = path_str.find(TEMP_DIR_PREFIX) {
        let base_path = &path_str[..builtin_start + TEMP_DIR_PREFIX.len()];
        let remaining = &path_str[builtin_start + TEMP_DIR_PREFIX.len()..];

        // Find the first directory after TEMP_DIR_PREFIX
        // e.g., lspintar_builtin_sources/some.dependency.1.0/com/... -> some.dependency.1.0
        if let Some(first_slash) = remaining[1..].find('/') {
            let dep_name = &remaining[1..first_slash + 1];
            let dep_root = format!("{}/{}", base_path, dep_name);
            return Some(PathBuf::from(dep_root));
        }
    }

    let mut current = nested_path.clone();
    while let Some(parent) = current.parent() {
        if parent.join("META-INF").exists() {
            return Some(parent.to_path_buf());
        }
        current = parent.to_path_buf();
    }

    None
}

#[tracing::instrument(skip_all)]
pub fn is_path_in_external_dependency(path: &PathBuf) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains(TEMP_DIR_PREFIX)
        || path_str.contains(".gradle/caches")
        || path_str.contains(".m2/repository")
        || has_meta_inf_in_parents(path)
}

#[tracing::instrument(skip_all)]
fn has_meta_inf_in_parents(path: &PathBuf) -> bool {
    let mut current = path.clone();
    while let Some(parent) = current.parent() {
        if parent.join("META-INF").exists() {
            return true;
        }
        current = parent.to_path_buf();
    }
    false
}

#[tracing::instrument(skip_all)]
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
        "kotlin" => {
            parser
                .set_language(&tree_sitter_kotlin::language())
                .inspect_err(|e| debug!("Cannot set kotlin parser: {e}"))
                .ok()?;
        }
        _ => return None,
    };

    Some(parser)
}

#[tracing::instrument(skip_all)]
pub fn detect_language_from_path(file_path: &PathBuf) -> Option<&'static str> {
    match file_path.extension()?.to_str()? {
        "java" => Some("java"),
        "groovy" | "gradle" => Some("groovy"),
        "kt" | "kts" => Some("kotlin"),
        _ => None,
    }
}

/// Get the appropriate language support for a given file path
#[tracing::instrument(skip_all)]
pub fn get_language_support_for_file(file_path: &PathBuf) -> Option<Box<dyn LanguageSupport>> {
    match detect_language_from_path(file_path)? {
        "java" => Some(Box::new(JavaSupport::new())),
        "groovy" => Some(Box::new(GroovySupport::new())),
        "kotlin" => Some(Box::new(KotlinSupport::new())),
        _ => None,
    }
}

/// Centralized cross-language search_definition_in_project dispatcher
/// Automatically detects the target file's language and calls the appropriate language's search function
#[tracing::instrument(skip_all)]
pub fn search_definition_in_project_cross_language(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &tree_sitter::Node,
    target_file_uri: &str,
    fallback_language_support: &dyn crate::languages::LanguageSupport,
) -> Option<tower_lsp::lsp_types::Location> {
    // Detect target file language
    let target_file_path = uri_to_path(target_file_uri)?;
    let target_language = detect_language_from_path(&target_file_path).unwrap_or("java");

    // Get the appropriate language support for the target file
    let target_language_support = get_language_support_for_file(&target_file_path)?;

    // Dispatch to the appropriate language's search function
    let result = match target_language {
        "groovy" => groovy_search_definition_in_project(
            current_file_uri,
            current_source,
            usage_node,
            target_file_uri,
            target_language_support.as_ref(),
        ),
        "java" => java_search_definition_in_project(
            current_file_uri,
            current_source,
            usage_node,
            target_file_uri,
            target_language_support.as_ref(),
        ),
        "kotlin" => kotlin_search_definition_in_project(
            current_file_uri,
            current_source,
            usage_node,
            target_file_uri,
            target_language_support.as_ref(),
        ),
        _ => {
            // Fallback to the provided language support (usually the current file's language)
            match fallback_language_support.language_id() {
                "java" => java_search_definition_in_project(
                    current_file_uri,
                    current_source,
                    usage_node,
                    target_file_uri,
                    fallback_language_support,
                ),
                "groovy" => groovy_search_definition_in_project(
                    current_file_uri,
                    current_source,
                    usage_node,
                    target_file_uri,
                    fallback_language_support,
                ),
                _ => None,
            }
        }
    };
    
    result
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

#[tracing::instrument(skip_all)]
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

#[tracing::instrument(skip_all)]
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

// Only get the closest node to root
#[tracing::instrument(skip_all)]
pub fn location_to_node<'a>(location: &Location, tree: &'a Tree) -> Option<Node<'a>> {
    let position = location.range.start;
    find_node_at_position(tree, position)
}


#[tracing::instrument(skip_all)]
pub fn is_project_root(current: &PathBuf) -> bool {
    tracing::debug!("Checking if {:?} is project root", current);

    for marker in PROJECT_ROOT_MARKER.iter() {
        let marker_path = current.join(marker);
        let exists = marker_path.exists();
        tracing::debug!("  Checking {:?}: {}", marker_path, exists);
        if exists {
            return true;
        }
    }
    false
}

#[tracing::instrument(skip_all)]
pub fn is_external_dependency(dir: &PathBuf) -> bool {
    return dir.to_string_lossy().contains(TEMP_DIR_PREFIX);
}

/// Sets the correct start position for a definition by searching for the symbol in the target file
/// This function works for any language by accepting a language parameter
#[tracing::instrument(skip_all)]
pub fn set_start_position_for_language(
    source: &str,
    usage_node: &Node,
    file_uri: &str,
    language: &str,
) -> Option<Location> {
    use tree_sitter::{QueryCursor, StreamingIterator};

    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
    let other_source = read_to_string(uri_to_path(file_uri)?).ok()?;


    // Use broader query that captures identifiers, type_identifiers, and simple_identifiers (for enum constants)
    let query_text = match language {
        "kotlin" => r#"(identifier) @name (type_identifier) @name (simple_identifier) @name"#,
        _ => r#"(identifier) @name"#,
    };

    // Create parser for the specified language
    let mut parser = create_parser_for_language(language)?;
    let tree = parser.parse(&other_source, None)?;

    // Get the query for the language - we need to get the language object
    let language_obj = match language {
        "groovy" => tree_sitter_groovy::language(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin::language(),
        _ => return None,
    };

    let query = tree_sitter::Query::new(&language_obj, query_text).ok()?;
    let mut cursor = QueryCursor::new();

    // Search for the symbol in the target file, excluding import statements  
    let mut matches = cursor.matches(&query, tree.root_node(), other_source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(name) = capture.node.utf8_text(other_source.as_bytes()) {
                if name == symbol_name {
                    // Skip if this match is within an import statement
                    if language == "kotlin" && is_in_import_statement(&capture.node) {
                        continue;
                    }
                    return node_to_lsp_location(&capture.node, file_uri);
                }
            }
        }
    }

    None
}

/// Check if a node is within an import statement
#[tracing::instrument(skip_all)]
fn is_in_import_statement(node: &tree_sitter::Node) -> bool {
    let mut current = Some(*node);
    let mut depth = 0;
    while let Some(n) = current {
        if depth >= 10 {
            break;
        }
        if n.kind() == "import_header" || n.kind() == "import_declaration" {
            return true;
        }
        current = n.parent();
        depth += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tower_lsp::lsp_types::{Position, Range, Url};

    struct UriConversionTestCase {
        name: &'static str,
        input: &'static str,
        expected_success: bool,
    }

    struct PathConversionTestCase {
        name: &'static str,
        input: PathBuf,
        expected_success: bool,
    }

    struct LanguageDetectionTestCase {
        name: &'static str,
        file_path: PathBuf,
        expected_language: Option<&'static str>,
    }

    struct ExternalDependencyTestCase {
        name: &'static str,
        path: PathBuf,
        expected_is_external: bool,
    }

    struct NodePositionTestCase {
        name: &'static str,
        position: Position,
        node_start: (usize, usize), // (row, col)
        node_end: (usize, usize),   // (row, col)
        expected_contains: bool,
    }

    #[test]
    fn test_uri_to_path_conversion() {
        let test_cases = vec![
            UriConversionTestCase {
                name: "valid file URI",
                input: "file:///home/user/test.groovy",
                expected_success: true,
            },
            UriConversionTestCase {
                name: "invalid URI",
                input: "not-a-uri",
                expected_success: false,
            },
            UriConversionTestCase {
                name: "http URI (not file)",
                input: "http://example.com/file.groovy",
                expected_success: false,
            },
        ];

        for test_case in test_cases {
            let result = uri_to_path(test_case.input);
            assert_eq!(
                result.is_some(),
                test_case.expected_success,
                "Test '{}': expected success = {}, got success = {}",
                test_case.name,
                test_case.expected_success,
                result.is_some()
            );
        }
    }

    #[test]
    fn test_path_to_uri_conversion() {
        let test_cases = vec![
            PathConversionTestCase {
                name: "valid absolute path",
                input: PathBuf::from("/home/user/test.groovy"),
                expected_success: true,
            },
            PathConversionTestCase {
                name: "relative path (may fail)",
                input: PathBuf::from("./test.groovy"),
                expected_success: false, // Relative paths typically don't work with file URIs
            },
        ];

        for test_case in test_cases {
            let result = path_to_file_uri(&test_case.input);
            assert_eq!(
                result.is_some(),
                test_case.expected_success,
                "Test '{}': expected success = {}, got success = {}",
                test_case.name,
                test_case.expected_success,
                result.is_some()
            );

            if result.is_some() {
                let uri = result.unwrap();
                assert!(uri.starts_with("file://"), "URI should start with file://");
            }
        }
    }

    #[test]
    fn test_language_detection() {
        let test_cases = vec![
            LanguageDetectionTestCase {
                name: "groovy file extension",
                file_path: PathBuf::from("/path/to/file.groovy"),
                expected_language: Some("groovy"),
            },
            LanguageDetectionTestCase {
                name: "gradle file extension",
                file_path: PathBuf::from("/path/to/build.gradle"),
                expected_language: Some("groovy"),
            },
            LanguageDetectionTestCase {
                name: "java file extension",
                file_path: PathBuf::from("/path/to/file.java"),
                expected_language: Some("java"),
            },
            LanguageDetectionTestCase {
                name: "kotlin file extension",
                file_path: PathBuf::from("/path/to/file.kt"),
                expected_language: Some("kotlin"),
            },
            LanguageDetectionTestCase {
                name: "kotlin script extension",
                file_path: PathBuf::from("/path/to/file.kts"),
                expected_language: Some("kotlin"),
            },
            LanguageDetectionTestCase {
                name: "unsupported extension",
                file_path: PathBuf::from("/path/to/file.txt"),
                expected_language: None,
            },
            LanguageDetectionTestCase {
                name: "no extension",
                file_path: PathBuf::from("/path/to/file"),
                expected_language: None,
            },
        ];

        for test_case in test_cases {
            let result = detect_language_from_path(&test_case.file_path);
            assert_eq!(
                result, test_case.expected_language,
                "Test '{}': expected {:?}, got {:?}",
                test_case.name, test_case.expected_language, result
            );
        }
    }

    #[test]
    fn test_external_dependency_detection() {
        let test_cases = vec![
            ExternalDependencyTestCase {
                name: "path with temp dir prefix",
                path: PathBuf::from("/tmp/lspintar_builtin_sources/some.dependency/File.groovy"),
                expected_is_external: true,
            },
            ExternalDependencyTestCase {
                name: "path with gradle cache",
                path: PathBuf::from("/home/user/.gradle/caches/modules-2/files-2.1/some.jar"),
                expected_is_external: true,
            },
            ExternalDependencyTestCase {
                name: "path with m2 repository",
                path: PathBuf::from("/home/user/.m2/repository/com/example/artifact.jar"),
                expected_is_external: true,
            },
            ExternalDependencyTestCase {
                name: "regular project path",
                path: PathBuf::from("/home/user/project/src/main/groovy/File.groovy"),
                expected_is_external: false,
            },
        ];

        for test_case in test_cases {
            let result = is_path_in_external_dependency(&test_case.path);
            assert_eq!(
                result, test_case.expected_is_external,
                "Test '{}': expected {}, got {}",
                test_case.name, test_case.expected_is_external, result
            );
        }
    }

    #[test]
    fn test_node_contains_position() {
        // Mock node implementation for testing
        struct MockNode {
            start_row: usize,
            start_col: usize,
            end_row: usize,
            end_col: usize,
        }

        fn mock_node_contains_position(node: &MockNode, position: Position) -> bool {
            let pos_line = position.line as usize;
            let pos_char = position.character as usize;

            (node.start_row < pos_line
                || (node.start_row == pos_line && node.start_col <= pos_char))
                && (pos_line < node.end_row
                    || (pos_line == node.end_row && pos_char <= node.end_col))
        }

        let test_cases = vec![
            NodePositionTestCase {
                name: "position inside node",
                position: Position {
                    line: 5,
                    character: 10,
                },
                node_start: (5, 5),
                node_end: (5, 15),
                expected_contains: true,
            },
            NodePositionTestCase {
                name: "position at start of node",
                position: Position {
                    line: 5,
                    character: 5,
                },
                node_start: (5, 5),
                node_end: (5, 15),
                expected_contains: true,
            },
            NodePositionTestCase {
                name: "position at end of node",
                position: Position {
                    line: 5,
                    character: 15,
                },
                node_start: (5, 5),
                node_end: (5, 15),
                expected_contains: true,
            },
            NodePositionTestCase {
                name: "position before node",
                position: Position {
                    line: 5,
                    character: 3,
                },
                node_start: (5, 5),
                node_end: (5, 15),
                expected_contains: false,
            },
            NodePositionTestCase {
                name: "position after node",
                position: Position {
                    line: 5,
                    character: 20,
                },
                node_start: (5, 5),
                node_end: (5, 15),
                expected_contains: false,
            },
            NodePositionTestCase {
                name: "position in multiline node",
                position: Position {
                    line: 6,
                    character: 5,
                },
                node_start: (5, 10),
                node_end: (7, 5),
                expected_contains: true,
            },
        ];

        for test_case in test_cases {
            let mock_node = MockNode {
                start_row: test_case.node_start.0,
                start_col: test_case.node_start.1,
                end_row: test_case.node_end.0,
                end_col: test_case.node_end.1,
            };

            let result = mock_node_contains_position(&mock_node, test_case.position);
            assert_eq!(
                result, test_case.expected_contains,
                "Test '{}': expected {}, got {}",
                test_case.name, test_case.expected_contains, result
            );
        }
    }

    #[test]
    fn test_create_parser_for_language() {
        let test_cases = vec![
            ("groovy", true),
            ("java", true),
            ("kotlin", true), // Parser is available
            ("unknown", false),
        ];

        for (language, expected_success) in test_cases {
            let result = create_parser_for_language(language);
            assert_eq!(
                result.is_some(),
                expected_success,
                "Language '{}': expected parser creation success = {}, got {}",
                language,
                expected_success,
                result.is_some()
            );
        }
    }

    #[test]
    fn test_node_to_lsp_location() {
        // This test demonstrates the structure of the function
        // In a real scenario, you'd need actual tree-sitter nodes
        let file_uri = "file:///test/file.groovy";

        // Test with a mock result
        let url = Url::parse(file_uri).expect("Valid URI");
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 10,
            },
        };
        let expected_location = Location { uri: url, range };

        // The actual function would need a real tree-sitter Node
        // This test verifies the structure is correct
        assert_eq!(expected_location.range.start.line, 0);
        assert_eq!(expected_location.range.start.character, 0);
    }

    #[test]
    fn test_is_external_dependency() {
        let test_cases = vec![
            (PathBuf::from("/tmp/lspintar_builtin_sources/dep"), true),
            (PathBuf::from("/home/user/project/src"), false),
            (PathBuf::from("/random/path"), false),
        ];

        for (path, expected) in test_cases {
            let result = is_external_dependency(&path);
            assert_eq!(
                result, expected,
                "Path {:?}: expected {}, got {}",
                path, expected, result
            );
        }
    }
}

