use core::panic;
use std::sync::Arc;

use anyhow::Result;
use log::debug;
use tower_lsp::lsp_types::{
    Diagnostic, Hover, HoverContents, Location, MarkupContent, MarkupKind, Position,
};
use tree_sitter::{Parser, Tree};

use crate::constants::LSP_NAME;
use crate::core::dependency_cache::DependencyCache;
use crate::core::symbols::SymbolType;
use crate::core::utils::location_to_node;
use crate::languages::traits::LanguageSupport;

use super::definition::find_definition_location;
use super::definition::utils::{determine_symbol_type_from_context, get_query_for_symbol_type};
use super::diagnostics::collect_syntax_errors;
use super::hover::class::extract_class_signature;
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
        position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {
        let identifier_node = find_identifier_at_position(tree, source, position)?;

        find_definition_location(tree, source, dependency_cache, uri, &identifier_node)
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

    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover> {
        let node = location_to_node(&location, tree)?;

        let symbol_type = determine_symbol_type_from_context(tree, &node, source).ok()?;

        let mut content = None;
        match symbol_type {
            SymbolType::Class => content = extract_class_signature(tree, source),
            SymbolType::Interface => {
                // TODO: implement this
            }
            SymbolType::Method => {
                // TODO: implement this
            }
            SymbolType::Field => {
                // TODO: implement this
            }
            _ => (),
        };

        debug!("content: {:#?}", content);

        content.and_then(|c| {
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: c,
                }),
                range: Some(location.range),
            })
        })
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
