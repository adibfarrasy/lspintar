use tower_lsp::lsp_types::Location;
use tree_sitter::Node;
use crate::core::utils::set_start_position_for_language;

pub fn set_start_position(
    source: &str,
    usage_node: &Node,
    file_uri: &str,
) -> Option<Location> {
    set_start_position_for_language(source, usage_node, file_uri, "kotlin")
}