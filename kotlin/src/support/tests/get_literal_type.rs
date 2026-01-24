#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_detect_literal_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class TestClass {
            fun testLiterals() {
                val intLit = 123
                val longLit = 123L
                val floatLit = 1.5f
                val doubleLit = 3.0
                val hexLit = 0xFF
                val binaryLit = 0b1010
                val boolTrue = true
                val boolFalse = false
                val strLit = "hello"
                val nullLit: Any? = null
                val charLit = 'a'
                val unsignedLit = 123u
                val longUnsignedLit = 123uL
                val floatWithoutF = 4.2  // This is Double in Kotlin
            }
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let test_cases = vec![
        ("123", Some("Int".to_string())),
        ("123L", Some("Long".to_string())),
        ("1.5f", Some("Float".to_string())),
        ("3.0", Some("Double".to_string())),
        ("0xFF", Some("Int".to_string())),
        ("0b1010", Some("Int".to_string())),
        ("true", Some("Boolean".to_string())),
        ("false", Some("Boolean".to_string())),
        ("\"hello\"", Some("String".to_string())),
        ("null", None),
        ("'a'", Some("Char".to_string())),
        ("123u", Some("UInt".to_string())),
        ("123uL", Some("ULong".to_string())),
        ("4.2", Some("Double".to_string())),
    ];
    for (literal, expected) in test_cases {
        let pos = find_position(content, literal);
        let literal_type = support.get_literal_type(&parsed.0, &parsed.1, &pos);
        assert_eq!(literal_type, expected, "Failed for literal: {}", literal);
    }
}
