#![allow(unused_imports)]

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

#[test]
fn test_detect_literal_type() {
    let support = GroovySupport::new();
    let content = r#"
        class TestClass {
            void testLiterals() {
                def mapLit = [key: 'value']
                def arrayLit = ['a', 'b', 'c']
                def intLit = 123
                def longLit = 123L
                def floatLit = 1.5f
                def doubleLit = 3.0
                def hexLit = 0xFF
                def binaryLit = 0b1010
                def boolTrue = true
                def boolFalse = false
                def strLit = "hello"
                def nullLit = null
                def regexLit = /pattern/
            }
        }
    "#;

    let parsed = support.parse_str(&content).expect("cannot parse content");

    let test_cases = vec![
        ("[key: 'value']", Some("Map".to_string())),
        ("['a', 'b', 'c']", Some("List".to_string())),
        ("123", Some("Integer".to_string())),
        ("123L", Some("Long".to_string())),
        ("1.5f", Some("Float".to_string())),
        ("3.0", Some("Double".to_string())),
        ("0xFF", Some("Integer".to_string())),
        ("0b1010", Some("Integer".to_string())),
        ("true", Some("Boolean".to_string())),
        ("false", Some("Boolean".to_string())),
        ("\"hello\"", Some("String".to_string())),
        ("null", None),
        ("/pattern/", Some("Pattern".to_string())),
    ];

    for (literal, expected) in test_cases {
        let pos = find_position(content, literal);
        let literal_type = support.get_literal_type(&parsed.0, &parsed.1, &pos);
        assert_eq!(literal_type, expected, "Failed for literal: {}", literal);
    }
}
