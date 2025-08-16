use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree};
use std::sync::Arc;
use anyhow::Result;

use crate::core::dependency_cache::DependencyCache;
use crate::languages::traits::LanguageSupport;

/// Enhanced definition resolution chain with cross-language support
pub fn find_definition_chain_generic(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Result<Location> {
    // Standard resolution chain: local -> project -> workspace -> external
    
    // 1. Try local resolution first
    if let Some(location) = super::local::find_local_generic(tree, source, file_uri, usage_node, language_support) {
        return Ok(location);
    }
    
    // 2. Try project-wide resolution
    if let Some(location) = super::project::find_in_project_generic(source, file_uri, usage_node, dependency_cache.clone(), language_support) {
        return Ok(location);
    }
    
    // 3. Try workspace-wide resolution
    if let Some(location) = super::workspace::find_in_workspace_generic(source, file_uri, usage_node, dependency_cache.clone(), language_support) {
        return Ok(location);
    }
    
    // 4. Try external dependencies
    if let Some(location) = super::external::find_external_generic(source, file_uri, usage_node, dependency_cache, language_support) {
        return Ok(location);
    }
    
    Err(anyhow::anyhow!("Definition not found"))
}

/// Cross-language definition resolution chain
pub fn find_definition_chain_cross_language(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    dependency_cache: Arc<DependencyCache>,
    registry: &crate::core::registry::LanguageRegistry,
    primary_language: &dyn LanguageSupport,
) -> Result<Location> {
    // 1. Try primary language first
    if let Ok(location) = find_definition_chain_generic(tree, source, file_uri, usage_node, dependency_cache.clone(), primary_language) {
        return Ok(location);
    }
    
    // 2. Try cross-language resolution
    // Check if this might be a cross-language reference (import analysis)
    if let Some(import_info) = extract_cross_language_info(usage_node, source, primary_language) {
        // Try to resolve using appropriate target language
        if let Some(location) = registry.resolve_cross_language_symbol(&import_info.symbol, &import_info.target_language, dependency_cache.clone()) {
            return Ok(location);
        }
    }
    
    // 3. Fallback: try all registered languages
    for language_id in registry.get_supported_languages() {
        if let Some(language) = registry.get_language(language_id) {
            if language.language_id() != primary_language.language_id() {
                if let Ok(location) = find_definition_chain_generic(tree, source, file_uri, usage_node, dependency_cache.clone(), language.as_ref()) {
                    return Ok(location);
                }
            }
        }
    }
    
    Err(anyhow::anyhow!("Definition not found in any language"))
}

/// Information about a potential cross-language reference
#[derive(Debug)]
pub struct CrossLanguageInfo {
    pub symbol: String,
    pub target_language: String,
    pub import_path: Option<String>,
}

/// Extract cross-language information from a usage node
fn extract_cross_language_info(
    usage_node: &Node,
    source: &str,
    language_support: &dyn LanguageSupport,
) -> Option<CrossLanguageInfo> {
    // TODO: Implement cross-language reference detection
    // This should:
    // 1. Look for import statements that suggest cross-language usage
    // 2. Analyze package names to determine target language
    // 3. Extract symbol and import path information
    
    None
}