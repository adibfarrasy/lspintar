#![allow(unused_imports)]

use crate::JavaSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_detect_literal_type() {
    let support = JavaSupport::new();
    let content = r#"
        class TestClass {
            void testLiterals() {
                int intLit = 123;
                long longLit = 123L;
                float floatLit = 1.5f;
                double doubleLit = 3.0;
                int hexLit = 0xFF;
                int binaryLit = 0b1010;
                boolean boolTrue = true;
                boolean boolFalse = false;
                String strLit = "hello";
                Object nullLit = null;
            }
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let test_cases = vec![
        ("123;", Some("Integer".to_string())),
        ("123L", Some("Long".to_string())),
        ("1.5f", Some("Float".to_string())),
        ("3.0", Some("Double".to_string())),
        ("0xFF", Some("Integer".to_string())),
        ("0b1010", Some("Integer".to_string())),
        ("true", Some("Boolean".to_string())),
        ("false", Some("Boolean".to_string())),
        ("\"hello\"", Some("String".to_string())),
        ("null", None),
    ];
    for (literal, expected) in test_cases {
        let pos = find_position(content, literal);
        let literal_type = support.get_literal_type(&parsed.0, &parsed.1, &pos);
        assert_eq!(literal_type, expected, "Failed for literal: {}", literal);
    }
}
