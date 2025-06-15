use std::sync::Arc;

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::core::{
    dependency_cache::{builtin::BuiltinTypeInfo, DependencyCache},
    symbols::SymbolType,
    utils::{node_to_lsp_location, path_to_file_uri},
};

use super::utils::search_definition;

#[tracing::instrument(skip_all)]
pub fn find_builtin(
    source: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let (symbol_name, builtin_info) =
        extract_symbol_and_lookup_builtin(source, usage_node, dependency_cache)?;

    search_builtin_definition_and_convert(&symbol_name, builtin_info)
}

#[tracing::instrument(skip_all)]
fn extract_symbol_and_lookup_builtin(
    source: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
) -> Option<(
    String,
    crate::core::dependency_cache::builtin::BuiltinTypeInfo,
)> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    let builtin_info = dependency_cache
        .builtin_infos
        .get(&symbol_name)?
        .value()
        .clone();

    Some((symbol_name, builtin_info))
}

#[tracing::instrument(skip_all)]
fn search_builtin_definition_and_convert(
    symbol_name: &str,
    builtin_info: BuiltinTypeInfo,
) -> Option<Location> {
    let symbol_type = SymbolType::Type;

    let definition_node = search_definition(
        &builtin_info.tree,
        &builtin_info.content,
        symbol_name,
        symbol_type,
    )?;

    let builtin_file_uri = get_builtin_uri(&builtin_info)?;
    node_to_lsp_location(&definition_node, &builtin_file_uri)
}

fn get_builtin_uri(builtin_info: &BuiltinTypeInfo) -> Option<String> {
    if let Some(zip_internal_path) = &builtin_info.zip_internal_path {
        extract_zip_file_to_temp(builtin_info, zip_internal_path)
    } else {
        path_to_file_uri(&builtin_info.source_path)
    }
}

fn extract_zip_file_to_temp(
    builtin_info: &BuiltinTypeInfo,
    zip_internal_path: &str,
) -> Option<String> {
    let temp_dir = std::env::temp_dir().join("lspintar_builtin_sources");
    if let Err(_) = std::fs::create_dir_all(&temp_dir) {
        tracing::error!("Failed to create temp directory for builtin sources");
        return None;
    }

    // Create safe filename from internal path (replace / with _)
    let safe_filename = zip_internal_path.replace('/', "_");
    let temp_file = temp_dir.join(&safe_filename);

    if !temp_file.exists() {
        if let Err(e) = std::fs::write(&temp_file, &builtin_info.content) {
            tracing::error!("Failed to write temp file for builtin: {}", e);
            return None;
        }
        tracing::debug!("Extracted builtin to temp file: {:?}", temp_file);
    }

    path_to_file_uri(&temp_file)
}
