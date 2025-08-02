use std::sync::Arc;

use anyhow::Result;
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Node, Parser, Tree};

use crate::core::{dependency_cache::DependencyCache, symbols::SymbolType};

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

    fn determine_symbol_type_from_context(
        &self,
        tree: &Tree,
        node: &Node,
        source: &str,
    ) -> Result<SymbolType>;

    fn find_definition_chain(
        &self,
        tree: &Tree,
        source: &str,
        dependency_cache: Arc<DependencyCache>,
        file_uri: &str,
        usage_node: &Node,
    ) -> Result<Location> {
        self.find_local(tree, source, file_uri, usage_node)
            .or_else(|| {
                self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| self.find_external(source, file_uri, usage_node, dependency_cache.clone()))
            .and_then(|location| {
                self.set_start_position(source, usage_node, &location.uri.to_string())
            })
            .ok_or_else(|| anyhow::anyhow!("Definition not found"))
    }

    fn find_local(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
    ) -> Option<Location> {
        None
    }

    fn find_in_project(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        None
    }

    fn find_in_workspace(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        None
    }

    fn find_external(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        None
    }

    fn set_start_position(
        &self,
        source: &str,
        usage_node: &Node,
        file_uri: &str,
    ) -> Option<Location> {
        None
    }
}
