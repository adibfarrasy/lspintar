use std::sync::Arc;

use anyhow::Result;
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Parser, Tree};

use crate::core::dependency_cache::DependencyCache;

pub trait LanguageSupport: Send + Sync {
    fn language_id(&self) -> &'static str;

    fn file_extensions(&self) -> &[&'static str];

    fn create_parser(&self) -> Parser;

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic>;

    fn find_definition(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location>;

    fn find_implementation(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>>;

    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover>;
}
