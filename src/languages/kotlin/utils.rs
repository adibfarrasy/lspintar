use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Tree};

pub fn find_identifier_at_position<'a>(tree: &'a Tree, source: &str, position: Position) -> Option<Node<'a>> {
    let byte_offset = position_to_byte_offset(source, position)?;
    use tracing::debug;
    debug!("Kotlin utils: Position {:?} converted to byte offset {}", position, byte_offset);
    
    if let Some(context) = get_context_around_offset(source, byte_offset, 10) {
        debug!("Kotlin utils: Context around byte {}: '{}'", byte_offset, context);
    }
    
    let root_node = tree.root_node();
    let containing_node = find_deepest_node_containing_offset(root_node, byte_offset);
    if let Some(node) = containing_node {
        let node_text = node.utf8_text(source.as_bytes()).unwrap_or("?");
        debug!("Kotlin utils: Deepest node containing byte {} is '{}' ({}) at bytes {}-{}", 
               byte_offset, node_text, node.kind(), node.start_byte(), node.end_byte());
    }
    
    let result = find_identifier_at_byte_offset(root_node, byte_offset);
    
    if result.is_none() {
        debug!("Kotlin utils: No identifier found at byte offset {}, trying nearby positions", byte_offset);
        for offset in [byte_offset.saturating_sub(1), byte_offset + 1, byte_offset.saturating_sub(2), byte_offset + 2] {
            if let Some(node) = find_identifier_at_byte_offset(root_node, offset) {
                debug!("Kotlin utils: Found identifier at nearby offset {}", offset);
                return Some(node);
            }
        }
    }
    
    result
}

fn get_context_around_offset(source: &str, byte_offset: usize, context_size: usize) -> Option<String> {
    let start = byte_offset.saturating_sub(context_size);
    let end = (byte_offset + context_size).min(source.len());
    source.get(start..end).map(|s| s.replace('\n', "\\n"))
}

fn find_deepest_node_containing_offset<'a>(node: Node<'a>, byte_offset: usize) -> Option<Node<'a>> {
    if node.start_byte() <= byte_offset && byte_offset < node.end_byte() {
        for child in node.children(&mut node.walk()) {
            if let Some(deeper) = find_deepest_node_containing_offset(child, byte_offset) {
                return Some(deeper);
            }
        }
        Some(node)
    } else {
        None
    }
}

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

fn find_identifier_at_byte_offset<'a>(node: Node<'a>, byte_offset: usize) -> Option<Node<'a>> {
    use tracing::debug;
    
    let is_identifier = matches!(node.kind(), "simple_identifier" | "type_identifier");
    if is_identifier && node.start_byte() <= byte_offset && byte_offset < node.end_byte() {
        debug!("Kotlin utils: Found exact {} match at bytes {}-{}", 
               node.kind(), node.start_byte(), node.end_byte());
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
        
        debug!("Kotlin utils: Checking {} at bytes {}-{}, distance from {} is {}", 
               node.kind(), node.start_byte(), node.end_byte(), byte_offset, distance);
        
        if distance <= 3 {
            debug!("Kotlin utils: Using nearby {} match", node.kind());
            return Some(node);
        }
    }

    None
}

pub fn extract_identifier_name(node: &Node, source: &str) -> Option<String> {
    if matches!(node.kind(), "simple_identifier" | "type_identifier") {
        let start = node.start_byte();
        let end = node.end_byte();
        Some(source[start..end].to_string())
    } else {
        None
    }
}

pub fn position_to_point(position: Position) -> tree_sitter::Point {
    tree_sitter::Point {
        row: position.line as usize,
        column: position.character as usize,
    }
}

pub fn point_to_position(point: tree_sitter::Point) -> Position {
    Position {
        line: point.row as u32,
        character: point.column as u32,
    }
}

pub fn node_to_range(node: &Node, source: &str) -> Range {
    Range {
        start: byte_to_position(source, node.start_byte()),
        end: byte_to_position(source, node.end_byte()),
    }
}

pub fn byte_to_position(source: &str, byte_offset: usize) -> Position {
    let mut line = 0;
    let mut character = 0;

    for (i, ch) in source.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}