use std::sync::Arc;
use anyhow::Result;
use tower_lsp::lsp_types::{Location, Position};
use tree_sitter::Tree;
use crate::{core::dependency_cache::DependencyCache, languages::LanguageSupport};

pub fn handle(
    _tree: &Tree,
    _source: &str,
    _position: Position,
    _dependency_cache: Arc<DependencyCache>,
    _language_support: &dyn LanguageSupport,
) -> Result<Vec<Location>> {
    // TODO: Implement Kotlin implementation finding
    // This would find implementations of interfaces/abstract methods
    Ok(Vec::new())
}