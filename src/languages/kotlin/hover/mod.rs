pub mod class;
pub mod field;
pub mod interface;
pub mod method;
pub mod utils;

use tower_lsp::lsp_types::{Hover, Location};
use tree_sitter::Tree;
use crate::languages::LanguageSupport;

pub fn handle(
    _tree: &Tree,
    _source: &str,
    _location: Location,
    _language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    // TODO: Implement Kotlin hover support
    // This would provide hover information for Kotlin symbols
    None
}