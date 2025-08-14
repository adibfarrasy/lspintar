use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tracing::{debug, error};
use tree_sitter::Node;

use crate::{
    core::{
        build_tools::ExternalDependency,
        constants::{IS_INDEXING_COMPLETED, TEMP_DIR_PREFIX},
        dependency_cache::{
            source_file_info::{self, SourceFileInfo},
            DependencyCache,
        },
        state_manager::get_global,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location,
            path_to_file_uri, uri_to_path,
        },
    },
    lsp_warning,
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition};

// FIXME: currently accidentally work because the tree-sitter node names overlap
// betweeen java and groovy.
#[tracing::instrument(skip_all)]
pub fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let current_project = uri_to_path(file_uri).and_then(|path| {
        find_project_root(&path).or_else(|| find_external_dependency_root(&path))
    })?;

    find_project_external(
        source,
        file_uri,
        usage_node,
        current_project,
        dependency_cache.clone(),
    )
}

#[tracing::instrument(skip_all)]
fn find_project_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    current_project: PathBuf,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Try to resolve the symbol through imports (including wildcard imports)
    let resolved_symbol = if let Some((_, fully_qualified_name)) = 
        prepare_symbol_lookup_key_with_wildcard_support(usage_node, source, file_uri, Some(current_project.clone()), &dependency_cache) {
        // Extract just the class name from the FQN for external lookup
        fully_qualified_name.split('.').last().unwrap_or(&symbol_name).to_string()
    } else {
        symbol_name.clone()
    };

    let project_key = (current_project, resolved_symbol.clone());
    if let Some(external_info) = dependency_cache.project_external_infos.get(&project_key) {
        return search_external_definition_and_convert(&symbol_name, external_info.value().clone());
    }
    if let Some(external_info) = dependency_cache.builtin_infos.get(&resolved_symbol) {
        return search_external_definition_and_convert(&symbol_name, external_info.value().clone());
    }

    if get_global(IS_INDEXING_COMPLETED).is_none() {
        lsp_warning!("Indexing still in progress...");
    }

    None
}

#[tracing::instrument(skip_all)]
fn search_external_definition_and_convert(
    symbol_name: &str,
    external_info: SourceFileInfo,
) -> Option<Location> {
    let tree = external_info
        .get_tree()
        .context(format!("failed to get tree for {symbol_name}"))
        .ok()?;

    let content = external_info
        .get_content()
        .context(format!("failed to get content for {symbol_name}"))
        .ok()?;

    let definition_node = search_definition(&tree, &content, symbol_name, SymbolType::Type)
        .context(format!("definition for {symbol_name} not found"))
        .ok()?;

    let file_uri = get_uri(&external_info.clone())
        .context(format!("file_uri for {symbol_name} not found"))
        .ok()?;

    node_to_lsp_location(&definition_node, &file_uri)
}

fn get_uri(external_info: &SourceFileInfo) -> Option<String> {
    if let Some(_) = &external_info.zip_internal_path {
        let temp_dir = dependency_temp_dir(external_info.dependency.clone());
        if !temp_dir.exists() {
            extract_zip_file_to_temp(external_info);
        }

        path_to_file_uri(&temp_dir.join(external_info.zip_internal_path.clone().unwrap()))
    } else {
        path_to_file_uri(&external_info.source_path)
    }
}

fn dependency_temp_dir(dependency: Option<ExternalDependency>) -> PathBuf {
    let base_dir = std::env::temp_dir().join(TEMP_DIR_PREFIX);

    match dependency {
        Some(dep) => base_dir.join(dep.to_path_string()),
        None => base_dir.join("builtin"),
    }
}

fn extract_zip_file_to_temp(builtin_info: &SourceFileInfo) -> Option<()> {
    let temp_dir = dependency_temp_dir(builtin_info.dependency.clone());

    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        error!("Failed to create temp directory: {}", e);
        return None;
    }

    let zip_file = std::fs::File::open(&builtin_info.source_path)
        .context(format!(
            "failed to open zip file {:?}",
            builtin_info.source_path
        ))
        .ok()?;

    let mut archive = zip::ZipArchive::new(zip_file)
        .context("failed to read zip archive")
        .ok()?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .context(format!("failed to get file at index {}", i))
            .ok()?;

        if file.is_dir() {
            continue;
        }

        let file_path = temp_dir.join(file.name());

        if let Some(parent) = file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(
                    "Failed to create directory structure for {:?}: {}",
                    parent, e
                );
                continue;
            }
        }

        let mut output = std::fs::File::create(&file_path)
            .context(format!("failed to create file {:?}", file_path))
            .ok()?;

        if let Err(e) = std::io::copy(&mut file, &mut output) {
            error!("Failed to extract file {}: {}", file.name(), e);
            continue;
        }
    }

    debug!("Extracted dependency to: {:?}", temp_dir);
    Some(())
}
