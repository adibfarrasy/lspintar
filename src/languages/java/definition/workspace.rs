use std::{collections::HashSet, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;
use tracing::debug;

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
pub async fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
    recursion_depth: usize,
) -> Option<Location> {
    let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    
    // FIRST: Check for nested enum access patterns (same as find_in_project)
    if let Some(parent) = usage_node.parent() {
        if parent.kind() == "field_access" {
            if let Some(enum_type_node) = parent.child_by_field_name("object") {
                if let Some(enum_type_name) = super::project::resolve_nested_type(source, &enum_type_node) {
                    if enum_type_name.contains('.') {
                        return super::project::find_nested_enum_using_regular_resolution(
                            source,
                            file_uri,
                            &enum_type_name,
                            symbol_text,
                            dependency_cache.clone(),
                            language_support,
                        ).await;
                    }
                }
            }
        }
    }

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
            recursion_depth,
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

    // Search using project metadata dependencies
    if let Some(project_metadata) = dependency_cache.project_metadata.get(current_project) {
        for dependent_project_ref in project_metadata.inter_project_deps.iter() {
            let dependent_project = dependent_project_ref.clone();

            // Try each search key (simple name + fully qualified imports)
            for search_key in &search_keys {
                let project_symbol_key = (dependent_project.clone(), search_key.clone());
                
                debug!(
                    "LSPINTAR_DEBUG: searching for ({:?}, '{}') in dependency_cache", 
                    project_symbol_key.0, project_symbol_key.1
                );

                let target_file_opt = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(
                        dependency_cache.find_symbol(&project_symbol_key.0, &project_symbol_key.1),
                    )
                });

                if let Some(target_file) = target_file_opt {
                    debug!(
                        "LSPINTAR_DEBUG: FOUND! target_file = {:?}", 
                        target_file
                    );

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
                        debug!(
                            "LSPINTAR_DEBUG: SUCCESSFULLY found definition at {:?}",
                            location
                        );
                        return Some(location);
                    } else {
                        debug!(
                            "LSPINTAR_DEBUG: cross-language search FAILED for target_uri = {}",
                            target_uri
                        );
                    }
                } else {
                    debug!(
                        "LSPINTAR_DEBUG: NOT FOUND - ({:?}, '{}')", 
                        project_symbol_key.0, project_symbol_key.1
                    );
                }
            }
        }
    } else {
        debug!(
            "LSPINTAR_DEBUG: no project_metadata found for {:?}", 
            current_project
        );
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
    recursion_depth: usize,
) -> Option<Location> {
    // Check recursion depth
    const MAX_RECURSION_DEPTH: usize = 10;
    if recursion_depth >= MAX_RECURSION_DEPTH {
        tracing::warn!("Maximum recursion depth {} reached in fallback_impl", MAX_RECURSION_DEPTH);
        return None;
    }
    
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

