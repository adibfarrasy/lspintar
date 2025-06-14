use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use method::extract_method_signature;
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::Tree;

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::groovy::definition::utils::determine_symbol_type_from_context,
};

mod class;
mod field;
mod interface;
mod method;
mod utils;

pub fn handle(tree: &Tree, source: &str, location: Location) -> Option<Hover> {
    let node = location_to_node(&location, tree)?;

    let symbol_type = determine_symbol_type_from_context(tree, &node, source).ok()?;

    let content = match symbol_type {
        SymbolType::SuperClass => extract_class_signature(tree, source),
        SymbolType::SuperInterface => extract_interface_signature(tree, source),
        SymbolType::MethodCall => extract_method_signature(tree, &node, source),
        SymbolType::FieldUsage => extract_field_signature(tree, &node, source),
        _ => None,
    };

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
