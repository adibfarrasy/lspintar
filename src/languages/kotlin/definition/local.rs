use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Tree};

use crate::{
    core::utils::node_to_lsp_location,
    languages::LanguageSupport,
};

pub fn find_local(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    // Search for declarations in the same file
    if let Some(definition_node) = search_local_definitions(tree, source, usage_node, symbol_name) {
        return node_to_lsp_location(&definition_node, file_uri);
    }

    None
}

fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node,
    symbol_name: &str,
) -> Option<Node<'a>> {
    // Simple approach: search for matching declarations in the same file
    search_for_declaration(tree.root_node(), source, symbol_name, usage_node.start_byte())
}

fn search_for_declaration<'a>(
    node: Node<'a>,
    source: &str,
    symbol_name: &str,
    usage_byte_offset: usize,
) -> Option<Node<'a>> {
    // Check if this node is a declaration that matches our symbol
    if is_declaration_node(&node) {
        if let Some(declared_name) = get_declared_name(&node, source) {
            if declared_name == symbol_name && node.start_byte() < usage_byte_offset {
                return Some(node);
            }
        }
    }

    // Search children
    for child in node.children(&mut node.walk()) {
        if let Some(result) = search_for_declaration(child, source, symbol_name, usage_byte_offset) {
            return Some(result);
        }
    }

    None
}

fn is_declaration_node(node: &Node) -> bool {
    matches!(
        node.kind(),
        "function_declaration" | "property_declaration" | "class_declaration" | "object_declaration" | "parameter" | "class_parameter"
    )
}

fn get_declared_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declaration" => {
            // Find the simple_identifier child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "class_declaration" | "object_declaration" => {
            // Find the type_identifier child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "type_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        "property_declaration" => {
            // Find variable_declaration -> simple_identifier
            for child in node.children(&mut node.walk()) {
                if child.kind() == "variable_declaration" {
                    for grandchild in child.children(&mut child.walk()) {
                        if grandchild.kind() == "simple_identifier" {
                            return grandchild.utf8_text(source.as_bytes()).ok().map(String::from);
                        }
                    }
                }
            }
        }
        "parameter" | "class_parameter" => {
            // Find the simple_identifier child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "simple_identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
            }
        }
        _ => {}
    }
    None
}