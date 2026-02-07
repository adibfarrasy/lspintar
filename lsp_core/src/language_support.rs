use std::path::Path;

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Tree};

use crate::{languages::Language, node_types::NodeType};

pub type ParseResult = (Tree, String);

// (name, qualifier)
pub type IdentResult = (String, Option<String>);

// (name, type_name, default_value)
pub type ParameterResult = (String, Option<String>, Option<String>);

pub trait LanguageSupport: Send + Sync {
    fn get_language(&self) -> Language;
    fn get_ts_language(&self) -> tree_sitter::Language;
    fn parse(&self, file_path: &Path) -> Option<ParseResult>;
    fn parse_str(&self, source: &str) -> Option<ParseResult>;

    fn should_index(&self, node: &Node, source: &str) -> bool {
        self.get_type(node).is_some()
    }

    fn get_range(&self, node: &Node) -> Option<Range>;
    fn get_ident_range(&self, node: &Node) -> Option<Range>;

    /*
     * Identifier
     */
    fn get_package_name(&self, tree: &Tree, source: &str) -> Option<String>;
    fn get_type(&self, node: &Node) -> Option<NodeType>;
    fn get_short_name(&self, node: &Node, source: &str) -> Option<String>;

    /*
     * Hierarchy
     */
    fn get_extends(&self, node: &Node, source: &str) -> Option<String>;
    fn get_implements(&self, node: &Node, source: &str) -> Vec<String>;

    /*
     * Metadata
     */
    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String>;
    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String>;
    fn get_documentation(&self, node: &Node, source: &str) -> Option<String>;
    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>>;
    fn get_return(&self, node: &Node, source: &str) -> Option<String>;

    // should also return implicit imports
    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String>;

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String>;

    fn find_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult>;

    fn find_variable_type(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<String>;

    // returns (type, position)
    fn find_variable_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(String, Position)>;

    fn extract_call_arguments(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<Vec<(String, Position)>>;

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String>;

    fn get_method_receiver_and_params(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<(String, Vec<String>)>;
}
