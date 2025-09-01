use tower_lsp::lsp_types::Position;
use tree_sitter::{Node, Tree};

#[tracing::instrument(skip_all)]
pub fn find_identifier_at_position<'a>(tree: &'a Tree, source: &str, position: Position) -> Option<Node<'a>> {
    let byte_offset = position_to_byte_offset(source, position)?;
    let root_node = tree.root_node();
    
    let result = find_identifier_at_byte_offset(root_node, byte_offset);
    
    if result.is_none() {
        for offset in [byte_offset.saturating_sub(1), byte_offset + 1, byte_offset.saturating_sub(2), byte_offset + 2] {
            if let Some(node) = find_identifier_at_byte_offset(root_node, offset) {
                return Some(node);
            }
        }
    }
    
    result
}



#[tracing::instrument(skip_all)]
fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let mut byte_offset = 0;
    let mut current_line = 0;
    let mut current_character = 0;

    for ch in source.chars() {
        if current_line == position.line && current_character == position.character {
            return Some(byte_offset);
        }

        if ch == '\n' {
            current_line += 1;
            current_character = 0;
        } else {
            current_character += ch.len_utf16() as u32;
        }

        byte_offset += ch.len_utf8();
    }

    if current_line == position.line && current_character == position.character {
        Some(byte_offset)
    } else {
        None
    }
}

#[tracing::instrument(skip_all)]
fn find_identifier_at_byte_offset<'a>(node: Node<'a>, byte_offset: usize) -> Option<Node<'a>> {
    let is_identifier = matches!(node.kind(), "simple_identifier" | "type_identifier");
    if is_identifier && node.start_byte() <= byte_offset && byte_offset < node.end_byte() {
        return Some(node);
    }

    for child in node.children(&mut node.walk()) {
        if child.start_byte() <= byte_offset && byte_offset < child.end_byte() {
            if let Some(result) = find_identifier_at_byte_offset(child, byte_offset) {
                return Some(result);
            }
        }
    }

    if is_identifier {
        let distance = if byte_offset < node.start_byte() {
            node.start_byte() - byte_offset
        } else {
            byte_offset - node.end_byte()
        };
        
        if distance <= 3 {
            return Some(node);
        }
    }

    None
}





