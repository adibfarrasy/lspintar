use anyhow::{Context, Result};
use log::debug;
use std::sync::{Arc, OnceLock};
use tokio::{fs, spawn, task};
use tower_lsp::lsp_types::{Location, Position};
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    dependency_cache::DependencyCache,
    symbols::SymbolType,
    utils::{create_parser_for_language, node_to_lsp_location, path_to_file_uri, uri_to_path},
};

use super::{
    definition::utils::determine_symbol_type_from_context, utils::find_identifier_at_position,
};

static IMPLEMENTATION_QUERY: OnceLock<Option<Query>> = OnceLock::new();

static IMPLEMENTATION_WITH_METHOD_QUERY: OnceLock<Option<Query>> = OnceLock::new();

static INTERFACE_DECLARATION_QUERY: OnceLock<Option<Query>> = OnceLock::new();

pub fn handle(
    tree: &Tree,
    source: &str,
    position: Position,
    dependency_cache: Arc<DependencyCache>,
) -> Result<Vec<Location>> {
    let identifier_node = find_identifier_at_position(tree, source, position)?;
    let symbol_name = identifier_node.utf8_text(source.as_bytes())?;
    let symbol_type = determine_symbol_type_from_context(tree, &identifier_node, source)?;

    match symbol_type {
        SymbolType::InterfaceDeclaration => futures::executor::block_on(
            find_interface_implementations(symbol_name, &dependency_cache),
        ),
        SymbolType::MethodDeclaration => futures::executor::block_on(async {
            // TODO: currently only handle interfaces.
            // implement abstract class handling next.

            let parent_name =
                get_parent_name(tree, source, symbol_name).context("Failed to get parent name")?;

            let locations = find_interface_implementations(&parent_name, &dependency_cache).await?;

            find_method_implementations(symbol_name, locations).await
        }),
        _ => Ok(vec![]),
    }
}

async fn find_interface_implementations(
    interface_name: &str,
    dependency_cache: &DependencyCache,
) -> Result<Vec<Location>> {
    // TODO: because it's always looping this has performance issue
    // implement caching import sites next

    let tasks: Vec<_> = dependency_cache
        .symbol_index
        .iter()
        .filter_map(|entry| {
            let (_, file_path) = (entry.key(), entry.value());
            path_to_file_uri(file_path).map(|file_uri| {
                let interface_name = interface_name.to_string();
                spawn(async move { find_implementations_in_file(&file_uri, &interface_name).await })
            })
        })
        .collect();

    let results = futures::future::join_all(tasks).await;

    debug!("I'm here");
    let mut implementations = Vec::new();
    for result in results {
        if let Ok(Ok(impls)) = result {
            implementations.extend(impls);
        }
    }

    Ok(implementations)
}

async fn find_implementations_in_file(file_uri: &str, target_name: &str) -> Result<Vec<Location>> {
    let file_path = uri_to_path(file_uri).context("Invalid file URI")?;
    let content = fs::read_to_string(&file_path)
        .await
        .context("Failed to read file")?;

    let file_uri = file_uri.to_string();
    let target_name = target_name.to_string();

    task::spawn_blocking(move || {
        let mut parser = create_parser_for_language("groovy").context("Failed to create parser")?;
        let tree = parser
            .parse(&content, None)
            .context("Failed to parse file")?;

        let query = get_implementation_query()
            .as_ref()
            .context("Failed to get query")?;
        let mut cursor = QueryCursor::new();
        let mut locations = Vec::new();

        cursor
            .matches(&query, tree.root_node(), content.as_bytes())
            .for_each(|query_match| {
                let mut class_name_node = None;
                let mut target_found = false;

                for capture in query_match.captures {
                    let capture_name = query.capture_names()[capture.index as usize];
                    let node_text = capture.node.utf8_text(content.as_bytes()).unwrap_or("");

                    match capture_name {
                        "class_name" => class_name_node = Some(capture.node),
                        "interface_name" if node_text == target_name => {
                            target_found = true;
                        }
                        _ => {}
                    }
                }

                if target_found {
                    if let Some(node) = class_name_node {
                        if let Some(location) = node_to_lsp_location(&node, &file_uri) {
                            locations.push(location);
                        }
                    }
                }
            });

        debug!("interface implementation locations: {:#?}", locations);
        Ok(locations)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
}

fn get_parent_name(tree: &Tree, source: &str, symbol_name: &str) -> Option<String> {
    let mut cursor = tree_sitter::QueryCursor::new();

    let query = get_interface_declaration_query()
        .as_ref()
        .context("Failed to get query")
        .ok()?;

    let mut interface_name = None;
    let mut found = false;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found {
                return;
            }

            for capture in query_match.captures {
                match capture.index {
                    0 => {
                        if let Ok(name) = capture.node.utf8_text(source.as_bytes()) {
                            interface_name = Some(name.to_string());
                        }
                    }
                    1 => {
                        if let Ok(method_name) = capture.node.utf8_text(source.as_bytes()) {
                            if method_name == symbol_name {
                                found = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

    interface_name
}

async fn find_method_implementations(
    symbol_name: &str,
    locations: Vec<Location>,
) -> Result<Vec<Location>> {
    // TODO: currently using naive implementation
    // should handle method overloading next.

    let mut results = Vec::new();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_groovy::language())?;

    let query = get_implementation_with_method_query()
        .as_ref()
        .context("Failed to get query")?;

    for location in locations {
        let content = fs::read_to_string(&location.uri.path()).await?;
        let tree = parser.parse(&content, None).context("failed to parse")?;
        let mut cursor = tree_sitter::QueryCursor::new();

        cursor
            .matches(&query, tree.root_node(), content.as_bytes())
            .for_each(|query_match| {
                for capture in query_match.captures {
                    if capture.index == 2 {
                        let method_name = capture.node.utf8_text(content.as_bytes()).unwrap_or("");
                        if method_name == symbol_name {
                            if let Some(loc) =
                                node_to_lsp_location(&capture.node, &location.uri.to_string())
                            {
                                results.push(loc);
                            }
                        }
                    }
                }
            })
    }

    Ok(results)
}

fn get_implementation_query() -> &'static Option<Query> {
    IMPLEMENTATION_QUERY.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        let text = r#"(class_declaration 
                name: (identifier) @class_name
                interfaces: (super_interfaces 
                    (type_list (type_identifier) @interface_name)))
                "#;

        Query::new(&language, text)
            .context("failed to parse query")
            .ok()
    })
}

fn get_implementation_with_method_query() -> &'static Option<Query> {
    IMPLEMENTATION_WITH_METHOD_QUERY.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        let text = r#"(class_declaration 
                name: (identifier) @class_name
                interfaces: (super_interfaces 
                    (type_list (type_identifier) @interface_name))
                body: (class_body
                    (method_declaration (identifier) @method_name))
                )"#;

        Query::new(&language, text)
            .context("failed to parse query")
            .ok()
    })
}

fn get_interface_declaration_query() -> &'static Option<Query> {
    INTERFACE_DECLARATION_QUERY.get_or_init(|| {
        let language = tree_sitter_groovy::language();
        let text = r#"
        (interface_declaration 
            name: (identifier) @interface_name
            body: (interface_body 
                (method_declaration name: (identifier) @method_name)))
        "#;

        Query::new(&language, text)
            .inspect_err(|error| debug!("[get_interface_declaration_query] {error}"))
            .ok()
    })
}
