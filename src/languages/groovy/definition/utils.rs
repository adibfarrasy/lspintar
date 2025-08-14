use std::{
    fs::{self, read_to_string},
    path::PathBuf,
};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
            uri_to_tree,
        },
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
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root
        .or_else(|| find_project_root(&current_file_path))
        .or_else(|| find_external_dependency_root(&current_file_path))?;

    resolve_through_imports(&symbol_name, source, &project_root)
        .or_else(|| resolve_same_package(&symbol_name, source, &project_root, dependency_cache))
}

/// Enhanced symbol lookup that supports wildcard imports using the class name index
pub fn prepare_symbol_lookup_key_with_wildcard_support(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root
        .or_else(|| find_project_root(&current_file_path))
        .or_else(|| find_external_dependency_root(&current_file_path))?;

    // First try regular resolution (specific imports and same package)
    let specific_import_result = resolve_through_imports(&symbol_name, source, &project_root);
    if let Some(result) = &specific_import_result {
        return Some(result.clone());
    }
    
    let same_package_result = resolve_same_package(&symbol_name, source, &project_root, dependency_cache);
    if let Some(result) = &same_package_result {
        return Some(result.clone());
    }

    // If not found, try wildcard import resolution
    resolve_through_wildcard_imports(&symbol_name, source, &project_root, dependency_cache)
}

fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (import_declaration) @import_decl
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut specific_import = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                    // Extract just the import path from "import com.example.package.*"
                    let import_text = full_import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    // Only handle specific imports here - wildcard imports are handled in resolve_through_wildcard_imports
                    if import_text.ends_with(&format!(".{}", symbol_name)) {
                        specific_import = Some((project_root.clone(), import_text.to_string()));
                        return;
                    }
                };
            }
        });

    // Return specific import if found - do NOT return wildcard candidates here
    specific_import
}

fn resolve_through_wildcard_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
    dependency_cache: &DependencyCache,
) -> Option<(PathBuf, String)> {
    // Get all wildcard imports from the source
    let wildcard_packages = get_wildcard_imports(source)?;
    
    // Get all possible FQNs for this class name in the project
    let possible_fqns = dependency_cache.find_symbols_by_class_name(project_root, symbol_name);
    
    // Check if any of the possible FQNs match any wildcard import
    for fqn in possible_fqns {
        for wildcard_package in &wildcard_packages {
            let expected_prefix = format!("{}.", wildcard_package);
            if fqn.starts_with(&expected_prefix) {
                return Some((project_root.clone(), fqn));
            }
        }
    }
    
    None
}

pub fn get_wildcard_imports_from_source(source: &str) -> Option<Vec<String>> {
    get_wildcard_imports(source)
}

fn get_wildcard_imports(source: &str) -> Option<Vec<String>> {
    let query_text = r#"
        (import_declaration) @import_decl
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut wildcard_packages = Vec::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                if let Ok(full_import_text) = capture.node.utf8_text(source.as_bytes()) {
                    // Extract just the import path from "import com.example.package.*"
                    let import_text = full_import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    if import_text.ends_with("*") {
                        let package_name = import_text.strip_suffix("*").unwrap_or(import_text);
                        let package_name = package_name.trim_end_matches('.');
                        wildcard_packages.push(package_name.to_string());
                    }
                }
            }
        });

    Some(wildcard_packages)
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
    dependency_cache: &DependencyCache,
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
                    
                    // Only return the same-package result if the symbol actually exists in the cache
                    let symbol_key = (project_root.clone(), fqn.clone());
                    if dependency_cache.symbol_index.get(&symbol_key).is_some() {
                        result = Some((project_root.clone(), fqn));
                        return;
                    }
                };
            }
        });

    result
}
