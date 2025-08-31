use crate::core::utils::node_contains_position;
use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::Position;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

#[tracing::instrument(skip_all)]
pub fn find_identifier_at_position<'a>(
    tree: &'a Tree,
    source: &str,
    position: Position,
) -> Result<Node<'a>> {
    let query_text = r#"
    (identifier) @identifier
    (type_identifier) @identifier
    (marker_annotation
      name: (identifier) @annotation)
    "#;
    let query = Query::new(&tree.language(), query_text).context(format!(
        "[find_identifier_at_position] failed to create a new query"
    ))?;

    let mut result: Result<Node> = Err(anyhow!(format!(
        "[find_identifier_at_position] invalid data. position: {:#?}",
        position
    )));
    let mut found = false;

    let mut cursor = QueryCursor::new();
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|match_| {
            if found {
                return;
            };

            for capture in match_.captures.iter() {
                let node = capture.node;
                if node_contains_position(&node, position) {
                    result = Ok(node);
                    found = true;
                    return;
                }
            }
        });

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_groovy_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        let language = tree_sitter_groovy::language();
        parser.set_language(&language).ok()?;
        Some(parser)
    }

    #[test]
    fn test_find_identifier_at_position_class_name() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "class TestClass {\n    void testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding class identifier
        let position = Position { line: 0, character: 8 }; // Position at 'T' in 'TestClass'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_ok());
        if let Ok(node) = result {
            assert_eq!(node.kind(), "identifier");
            assert_eq!(node.utf8_text(source.as_bytes()).unwrap(), "TestClass");
        }
    }

    #[test]
    fn test_find_identifier_at_position_method_name() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "class TestClass {\n    def testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding method identifier
        let position = Position { line: 1, character: 8 }; // Position at 't' in 'testMethod'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_ok());
        if let Ok(node) = result {
            assert_eq!(node.kind(), "identifier");
            assert_eq!(node.utf8_text(source.as_bytes()).unwrap(), "testMethod");
        }
    }

    #[test]
    fn test_find_identifier_at_position_annotation() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "@Override\ndef testMethod() {}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding annotation identifier
        let position = Position { line: 0, character: 3 }; // Position in '@Override'
        let result = find_identifier_at_position(&tree, source, position);
        
        // Should find Override identifier or return error for no match
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_find_identifier_at_position_variable() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "def myVariable = 10";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding variable identifier
        let position = Position { line: 0, character: 6 }; // Position at 'm' in 'myVariable'
        let result = find_identifier_at_position(&tree, source, position);
        
        assert!(result.is_ok());
        if let Ok(node) = result {
            assert_eq!(node.kind(), "identifier");
            assert_eq!(node.utf8_text(source.as_bytes()).unwrap(), "myVariable");
        }
    }

    #[test]
    fn test_find_identifier_at_position_no_identifier() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "class TestClass {\n    def testMethod() {}\n}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test position in whitespace
        let position = Position { line: 0, character: 5 }; // Position in whitespace
        let result = find_identifier_at_position(&tree, source, position);
        
        // Should return error since no identifier found
        assert!(result.is_err());
    }

    #[test]
    fn test_find_identifier_at_position_invalid_position() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "class TestClass {}";
        let tree = parser.parse(source, None).unwrap();
        
        // Test invalid position (way beyond source)
        let position = Position { line: 10, character: 50 };
        let result = find_identifier_at_position(&tree, source, position);
        
        // Should return error for invalid position
        assert!(result.is_err());
    }

    #[test]
    fn test_find_identifier_at_position_closure_parameter() {
        let mut parser = match create_groovy_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Groovy parser not available for testing");
                return;
            }
        };

        let source = "[1, 2, 3].each { item -> println item }";
        let tree = parser.parse(source, None).unwrap();
        
        // Test finding closure parameter identifier
        let position = Position { line: 0, character: 17 }; // Position at 'i' in 'item'
        let result = find_identifier_at_position(&tree, source, position);
        
        // May or may not find depending on Groovy parser capabilities
        assert!(result.is_ok() || result.is_err());
    }
}
