use core::panic;
use std::sync::Arc;

use anyhow::Result;
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Parser, Tree};

use crate::constants::LSP_NAME;
use crate::core::dependency_cache::DependencyCache;
use crate::languages::traits::LanguageSupport;

use super::definition;
use super::diagnostics::collect_syntax_errors;
use super::hover;
use super::implementation;
use super::utils::find_identifier_at_position;

pub struct GroovySupport;

impl GroovySupport {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageSupport for GroovySupport {
    fn language_id(&self) -> &'static str {
        "groovy"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &[".groovy", ".gradle", ".gvy", ".gy", ".gsh"]
    }

    fn create_parser(&self) -> Parser {
        let mut parser = Parser::new();
        if let Err(e) = parser.set_language(&tree_sitter_groovy::language()) {
            eprintln!("Warning: Failed to load Groovy grammar: {:?}", e);
            panic!("cannot load groovy grammar")
        }
        parser
    }

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic> {
        // TODO: replace this with more sophisticated handling
        collect_syntax_errors(tree, source, LSP_NAME)
    }

    fn find_definition(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {
        let identifier_node = find_identifier_at_position(tree, source, position)?;

        definition::handle(tree, source, dependency_cache, uri, &identifier_node)
    }

    fn find_implementation(
        &self,
        tree: &Tree,
        source: &str,
        position: tower_lsp::lsp_types::Position,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>> {
        implementation::handle(tree, source, position, dependency_cache)
    }

    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover> {
        hover::handle(tree, source, location)
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
