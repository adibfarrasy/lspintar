use std::{
    fs::{self, read_to_string},
    path::PathBuf,
};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        symbols::SymbolType,
        utils::{find_project_root, node_to_lsp_location, uri_to_path, uri_to_tree},
    },
    languages::LanguageSupport,
};

#[tracing::instrument(skip_all)]
pub fn get_declaration_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration name: (identifier) @name)
            (interface_declaration name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
            (annotation_type_declaration name: (identifier) @name)
        "#,
        ),
        SymbolType::SuperClass => Some(r#"(class_declaration name: (identifier) @name)"#),
        SymbolType::SuperInterface => Some(r#"(interface_declaration name: (identifier) @name)"#),
        SymbolType::MethodCall => Some(r#"(method_declaration name: (identifier) @name)"#),
        SymbolType::FieldUsage => Some(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        ),
        SymbolType::VariableUsage => Some(
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
        "#,
        ),
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
pub fn find_definition_candidates<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    query_text: &str,
) -> Option<Vec<Node<'a>>> {
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let node_text = capture.node.utf8_text(source.as_bytes()).unwrap();

                if node_text == symbol_name {
                    candidates.push(capture.node.parent().unwrap());
                }
            }
        });

    Some(candidates)
}

#[tracing::instrument(skip_all)]
pub fn search_definition<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    symbol_type: SymbolType,
) -> Option<Node<'a>> {
    let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;

    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;

    candidates.into_iter().next()
}

#[tracing::instrument(skip_all)]
pub fn search_definition_in_project(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &Node,
    other_file_uri: &str,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;
    let symbol_type = language_support
        .determine_symbol_type_from_context(&current_tree, usage_node, current_source)
        .ok()?;

    debug!("other_file_uri: {other_file_uri}");
    let other_tree = uri_to_tree(other_file_uri)?;
    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    let definition_node = search_definition(&other_tree, &other_source, symbol_name, symbol_type)?;

    return node_to_lsp_location(&definition_node, &other_file_uri);
}

#[tracing::instrument(skip_all)]
pub fn prepare_symbol_lookup_key(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root.or_else(|| find_project_root(&current_file_path))?;

    resolve_through_imports(&symbol_name, source, &project_root)
        .or_else(|| resolve_same_package(&symbol_name, source, &project_root))
}

fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (import_declaration
          (scoped_identifier) @import_name) 
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                return; // Already found a match
            }

            for capture in query_match.captures {
                if let Ok(import_text) = capture.node.utf8_text(source.as_bytes()) {
                    if import_text.ends_with(&format!(".{}", symbol_name)) {
                        debug!(
                            "project: {}, import_text: {import_text}, symbol_name: {symbol_name}",
                            project_root.to_str().unwrap()
                        );

                        result = Some((project_root.clone(), import_text.to_string()));
                        return;
                    }

                    if import_text.ends_with("*") {
                        // TODO: handle wildcard import
                    }
                };
            }
        });

    result
}

pub fn set_start_position(source: &str, usage_node: &Node, file_uri: &str) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    let other_source = fs::read_to_string(uri_to_path(file_uri)?).ok()?;

    let query_text = r#"
      (identifier) @name 
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&other_source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), other_source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                // Already found a match
                return;
            }

            for capture in query_match.captures {
                if let Ok(name) = capture.node.utf8_text(other_source.as_bytes()) {
                    if name == symbol_name {
                        result = node_to_lsp_location(&capture.node, file_uri)
                    }
                };
            }
        });

    result
}

fn resolve_same_package(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (package_declaration
          (scoped_identifier) @package_name)
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                // Already found a match
                // should only have 1 match
                return;
            }

            for capture in query_match.captures {
                if let Ok(package_name) = capture.node.utf8_text(source.as_bytes()) {
                    let fqn = format!("{}.{}", package_name, symbol_name);
                    result = Some((project_root.clone(), fqn));
                    return;
                };
            }
        });

    result
}
