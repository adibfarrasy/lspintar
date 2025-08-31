use std::{collections::HashSet, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{
            find_project_root, path_to_file_uri, search_definition_in_project_cross_language,
            uri_to_path,
        },
    },
    languages::LanguageSupport,
};

use super::utils::{
    extract_imports_from_source, get_wildcard_imports_from_source,
    prepare_symbol_lookup_key_with_wildcard_support, search_definition_in_project,
};

#[tracing::instrument(skip_all)]
pub fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let current_project = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;

    find_in_project_dependencies(
        source,
        file_uri,
        usage_node,
        &current_project,
        dependency_cache.clone(),
        language_support,
    )
    .or_else(|| {
        fallback_impl(
            source,
            file_uri,
            usage_node,
            dependency_cache,
            language_support,
        )
    })
}

#[tracing::instrument(skip_all)]
fn find_in_project_dependencies(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    current_project: &PathBuf,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_key = prepare_symbol_lookup_key_with_wildcard_support(
        usage_node,
        source,
        file_uri,
        None,
        &dependency_cache,
    )?;


    // Extract imports to look for fully qualified names
    let symbol_name = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    let imports = extract_imports_from_source(source);
    let mut search_keys = vec![symbol_key.1.clone()]; // Start with the resolved symbol key

    // Add fully qualified import names that match our symbol
    for import in &imports {
        if import.ends_with(&format!(".{}", symbol_name)) {
            search_keys.push(import.clone());
        }
    }


    // Get all projects in the workspace except the current one
    let mut checked_projects = HashSet::new();
    checked_projects.insert(current_project.clone());

    // Search in other projects in the workspace
    for entry in dependency_cache.symbol_index.iter() {
        let ((project_root, _), _file_path) = (entry.key(), entry.value());

        if !checked_projects.contains(project_root) {
            checked_projects.insert(project_root.clone());

            // Try each search key (simple name + fully qualified imports)
            for search_key in &search_keys {
                let project_symbol_key = (project_root.clone(), search_key.clone());

                let target_file_opt = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(
                        dependency_cache.find_symbol(&project_symbol_key.0, &project_symbol_key.1),
                    )
                });

                if let Some(target_file) = target_file_opt {

                    let target_uri = match path_to_file_uri(&target_file) {
                        Some(uri) => {
                            uri
                        }
                        None => {
                            continue;
                        }
                    };


                    // Use the centralized cross-language dispatcher
                    if let Some(location) = search_definition_in_project_cross_language(
                        file_uri,
                        source,
                        usage_node,
                        &target_uri,
                        language_support,
                    ) {
                        return Some(location);
                    } else {
                    }
                } else {
                }
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn fallback_impl(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    // Fallback: try to resolve using wildcard imports
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
    let _current_project = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;

    // Get wildcard imports from the current file
    let wildcard_imports = get_wildcard_imports_from_source(source);

    for import_package in wildcard_imports {
        let full_symbol_name = format!("{}.{}", import_package, symbol_name);

        // Search in all projects for this fully qualified name
        for entry in dependency_cache.symbol_index.iter() {
            let ((_project_root, fqn), file_path) = (entry.key(), entry.value());

            if fqn == &full_symbol_name {
                let target_uri = path_to_file_uri(&file_path)?;

                if let Some(location) = search_definition_in_project(
                    file_uri,
                    source,
                    usage_node,
                    &target_uri,
                    language_support,
                ) {
                    return Some(location);
                }
            }
        }
    }

    None
}

