use std::{path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tracing::debug;
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
    get_wildcard_imports_from_source, prepare_symbol_lookup_key_with_wildcard_support,
};

#[tracing::instrument(skip_all)]
pub async fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    
    // FIRST: Check for nested enum access patterns (same as find_in_project)
    
    if let Some(parent) = usage_node.parent() {
        if parent.kind() == "field_access" {
            if let Some(enum_type_node) = parent.child_by_field_name("object") {
                if let Some(enum_type_name) = super::project::resolve_nested_enum_type(source, &enum_type_node) {
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
        )
    })
}

#[tracing::instrument(skip_all)]
fn find_in_project_dependencies(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    current_project: &PathBuf,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let project_meta = dependency_cache.project_metadata.get(current_project)?;

    let symbol_key = prepare_symbol_lookup_key_with_wildcard_support(
        usage_node,
        source,
        file_uri,
        None,
        &dependency_cache,
    )?;
    let (_, fully_qualified_name) = symbol_key;

    // Search in each project that this project depends on
    for dep_project_root in project_meta.inter_project_deps.iter() {
        let dep_symbol_key = (dep_project_root.clone(), fully_qualified_name.clone());

        let file_location_opt = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(dependency_cache.find_symbol(&dep_symbol_key.0, &dep_symbol_key.1))
        });

        if let Some(file_location) = file_location_opt {
            let other_uri = path_to_file_uri(&file_location)?;

            // Use the centralized cross-language dispatcher
            if let Some(location) = search_definition_in_project_cross_language(
                file_uri,
                source,
                usage_node,
                &other_uri,
                language_support,
            ) {
                return Some(location);
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn fallback_impl(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    // Enhanced implementation that only searches valid project dependencies
    debug!("using fallback method");
    
    let current_project = uri_to_path(file_uri)?;
    let current_project_root = find_project_root(&current_project)?;
    
    // Get project metadata to find valid dependencies
    let project_metadata = dependency_cache.project_metadata.get(&current_project_root)?;
    
    // Only search through projects that are actual dependencies
    for dependent_project_ref in project_metadata.inter_project_deps.iter() {
        let project_root = dependent_project_ref.clone();

        let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

        // First try to resolve the symbol using import resolution for this project
        let symbol_key_from_imports = prepare_symbol_lookup_key_with_wildcard_support(
            usage_node,
            source,
            file_uri,
            Some(project_root.clone()),
            &dependency_cache,
        );

        if let Some((_, fqn)) = symbol_key_from_imports {
            let symbol_key = (project_root.clone(), fqn.clone());

            let file_location_opt = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(dependency_cache.find_symbol(&symbol_key.0, &symbol_key.1))
            });

            if let Some(file_location) = file_location_opt {
                let other_uri = path_to_file_uri(&file_location)?;

                // Use the centralized cross-language dispatcher
                return search_definition_in_project_cross_language(
                    file_uri,
                    source,
                    usage_node,
                    &other_uri,
                    language_support,
                );
            }
        }

        // Also try wildcard imports
        let wildcard_packages = get_wildcard_imports_from_source(source);
        if let Some(packages) = wildcard_packages {
            for package in packages {
                let candidate_fqn = format!("{}.{}", package, symbol_name);
                let symbol_key = (project_root.clone(), candidate_fqn.clone());

                let file_location_opt = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(dependency_cache.find_symbol(&symbol_key.0, &symbol_key.1))
                });

                if let Some(file_location) = file_location_opt {
                    let other_uri = path_to_file_uri(&file_location)?;

                    // Use the centralized cross-language dispatcher
                    return search_definition_in_project_cross_language(
                        file_uri,
                        source,
                        usage_node,
                        &other_uri,
                        language_support,
                    );
                }
            }
        }
    }

    None
}
