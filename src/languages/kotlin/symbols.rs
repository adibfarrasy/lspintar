use tree_sitter::{Node, Tree};
use std::collections::HashMap;
use crate::core::symbols::SymbolType;

pub struct KotlinSymbolCollector;

impl KotlinSymbolCollector {
    pub fn new() -> Self {
        Self
    }

    pub fn collect_symbols(&self, _tree: &Tree, _source: &str) -> HashMap<String, Vec<SymbolInfo>> {
        // TODO: Implement symbol collection when tree_sitter_kotlin is available
        HashMap::new()
    }
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub symbol_type: SymbolType,
    pub node: Node<'static>,
    pub range: tower_lsp::lsp_types::Range,
}