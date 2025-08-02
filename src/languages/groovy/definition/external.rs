use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tracing::{debug, error};
use tree_sitter::Node;

use crate::core::{
    dependency_cache::{external::SourceFileInfo, DependencyCache},
    symbols::SymbolType,
    utils::{find_project_root, node_to_lsp_location, path_to_file_uri, uri_to_path},
};

use super::utils::search_definition;

// FIXME: currently accidentally work because the tree-sitter node names overlap
// betweeen java and groovy.
#[tracing::instrument(skip_all)]
pub fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let current_project = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;

    find_project_external(
        source,
        usage_node,
        current_project,
        dependency_cache.clone(),
    )
    .or_else(|| fallback_impl(source, usage_node, dependency_cache))
}

#[tracing::instrument(skip_all)]
fn find_project_external(
    source: &str,
    usage_node: &Node,
    current_project: PathBuf,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let project_key = (current_project, symbol_name.clone());
    if let Some(external_info) = dependency_cache.project_external_infos.get(&project_key) {
        return search_external_definition_and_convert(&symbol_name, external_info.value().clone());
    }

    if let Some(external_info) = dependency_cache.external_infos.get(&symbol_name) {
        return search_external_definition_and_convert(&symbol_name, external_info.value().clone());
    }

    None
}

#[tracing::instrument(skip_all)]
fn fallback_impl(
    source: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let (symbol_name, external_info) =
        extract_symbol_and_external_info(source, usage_node, dependency_cache)?;

    search_external_definition_and_convert(&symbol_name, external_info)
}

#[tracing::instrument(skip_all)]
fn extract_symbol_and_external_info(
    source: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<(String, SourceFileInfo)> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let external_info = dependency_cache
        .external_infos
        .get(&symbol_name)?
        .value()
        .clone();

    Some((symbol_name, external_info))
}

#[tracing::instrument(skip_all)]
fn search_external_definition_and_convert(
    symbol_name: &str,
    external_info: SourceFileInfo,
) -> Option<Location> {
    let definition_node = search_definition(
        &external_info.tree,
        &external_info.content,
        symbol_name,
        SymbolType::Type,
    )
    .context(format!("definition for {symbol_name} not found"))
    .ok()?;

    let file_uri = get_uri(&external_info)
        .context(format!("file_uri for {symbol_name} not found"))
        .ok()?;

    debug!("file_uri: {:#?}", file_uri);

    node_to_lsp_location(&definition_node, &file_uri)
}

fn get_uri(external_info: &SourceFileInfo) -> Option<String> {
    if let Some(zip_internal_path) = &external_info.zip_internal_path {
        extract_zip_file_to_temp(external_info, zip_internal_path)
    } else {
        path_to_file_uri(&external_info.source_path)
    }
}

fn extract_zip_file_to_temp(
    builtin_info: &SourceFileInfo,
    zip_internal_path: &str,
) -> Option<String> {
    let temp_dir = std::env::temp_dir().join("lspintar_builtin_sources");
    if let Err(_) = std::fs::create_dir_all(&temp_dir) {
        error!("Failed to create temp directory for builtin sources");
        return None;
    }

    // Create safe filename from internal path (replace / with _)
    let safe_filename = zip_internal_path.replace('/', "_");
    let temp_file = temp_dir.join(&safe_filename);

    if !temp_file.exists() {
        if let Err(e) = std::fs::write(&temp_file, &builtin_info.content) {
            error!("Failed to write temp file for builtin: {}", e);
            return None;
        }
        debug!("Extracted builtin to temp file: {:?}", temp_file);
    }

    path_to_file_uri(&temp_file)
}
