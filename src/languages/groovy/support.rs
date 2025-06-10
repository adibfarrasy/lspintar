use core::panic;
use std::sync::Arc;

use anyhow::Result;
use tower_lsp::lsp_types::{Diagnostic, Hover, Location};
use tree_sitter::{Parser, Tree};

use crate::constants::LSP_NAME;
use crate::core::dependency_cache::DependencyCache;
use crate::languages::traits::LanguageSupport;

use super::definition::find_definition_location;
use super::diagnostics::collect_syntax_errors;
use super::implementation::find_implementations;
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
        position: tower_lsp::lsp_types::Position,
        file_uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {
        let identifier_node = find_identifier_at_position(tree, source, position)?;

        find_definition_location(tree, source, dependency_cache, file_uri, &identifier_node)
    }

    fn find_implementation(
        &self,
        tree: &Tree,
        source: &str,
        position: tower_lsp::lsp_types::Position,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>> {
        find_implementations(tree, source, position, dependency_cache)
    }

    fn provide_hover(
        &self,
        tree: &Tree,
        source: &str,
        position: tower_lsp::lsp_types::Position,
    ) -> Option<Hover> {
        todo!()
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
