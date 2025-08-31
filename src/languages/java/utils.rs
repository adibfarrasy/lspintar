use tower_lsp::lsp_types::Position;
use tree_sitter::{Node, Tree};

pub fn find_identifier_at_position<'a>(tree: &'a Tree, source: &str, position: Position) -> Option<Node<'a>> {
    let byte_offset = position_to_byte_offset(source, position)?;
    let root_node = tree.root_node();
    
    let result = find_identifier_at_byte_offset(root_node, byte_offset);
    
    if result.is_none() {
        // Try a few bytes around the position in case of UTF-8 boundary issues
        for offset in [byte_offset.saturating_sub(1), byte_offset + 1, byte_offset.saturating_sub(2), byte_offset + 2] {
            if let Some(node) = find_identifier_at_byte_offset(root_node, offset) {
                return Some(node);
            }
        }
    }
    
    result
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
    let is_identifier = matches!(node.kind(), "identifier" | "type_identifier");
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

    // If we're within 3 bytes, consider it a match (for cursor positioning tolerance)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_java_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        let language = tree_sitter_java::LANGUAGE;
        parser.set_language(&language.into()).ok()?;
        Some(parser)
    }

    #[test]
    fn test_position_to_byte_offset_single_line() {
        let source = "public class Test {}";
        let position = Position { line: 0, character: 6 }; // Position at 'c' in 'class'
        
        let result = position_to_byte_offset(source, position);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn test_position_to_byte_offset_multi_line() {
        let source = "public class Test {\n    void method() {\n    }\n}";
        let position = Position { line: 1, character: 4 }; // Position at 'v' in 'void'
        
        let result = position_to_byte_offset(source, position);
        assert_eq!(result, Some(24));
    }

    #[test]
    fn test_position_to_byte_offset_end_of_line() {
        let source = "public class Test {\n}";
        let position = Position { line: 0, character: 19 }; // End of first line
        
        let result = position_to_byte_offset(source, position);
        assert_eq!(result, Some(19));
    }

    #[test]
    fn test_position_to_byte_offset_invalid_position() {
        let source = "public class Test {}";
        let position = Position { line: 5, character: 0 }; // Line doesn't exist
        
        let result = position_to_byte_offset(source, position);
        assert_eq!(result, None);
    }

    #[test]
    fn test_position_to_byte_offset_utf8() {
        let source = "class 测试 {}";
        let position = Position { line: 0, character: 6 }; // Position at first UTF-8 char
        
        let result = position_to_byte_offset(source, position);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn test_find_identifier_at_position_with_valid_java_code() {
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };

        let source = "public class TestClass {\n    void testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding class identifier
        let position = Position { line: 0, character: 13 }; // Position at 'T' in 'TestClass'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_some());
        if let Some(node) = result {
            assert_eq!(node.kind(), "identifier");
            assert_eq!(node.utf8_text(source.as_bytes()).unwrap(), "TestClass");
        }
    }

    #[test]
    fn test_find_identifier_at_position_method_name() {
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };

        let source = "public class TestClass {\n    void testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding method identifier
        let position = Position { line: 1, character: 9 }; // Position at 't' in 'testMethod'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_some());
        if let Some(node) = result {
            assert_eq!(node.kind(), "identifier");
            assert_eq!(node.utf8_text(source.as_bytes()).unwrap(), "testMethod");
        }
    }

    #[test]
    fn test_find_identifier_at_position_no_identifier() {
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };

        let source = "public class TestClass {\n    void testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test position in whitespace
        let position = Position { line: 0, character: 5 }; // Position in whitespace after 'public'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_none());
    }

    #[test]
    fn test_find_identifier_at_byte_offset_tolerance() {
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };

        let source = "class Test {}";
        let tree = parser.parse(source, None).unwrap();
        let root_node = tree.root_node();
        
        // Find the identifier node for 'Test'
        let class_decl = root_node.child(0).unwrap();
        let mut cursor = class_decl.walk();
        let mut identifier_node = None;
        
        for child in class_decl.children(&mut cursor) {
            if child.kind() == "identifier" {
                identifier_node = Some(child);
                break;
            }
        }
        
        if let Some(id_node) = identifier_node {
            // Test tolerance - should find identifier even when slightly off
            let result = find_identifier_at_byte_offset(id_node, id_node.end_byte() + 1);
            assert!(result.is_some());
        }
    }
}

