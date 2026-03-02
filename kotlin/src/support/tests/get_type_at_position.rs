#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_kotlin_type_field_declaration() {
    let support = KotlinSupport::new();
    let content = r#"
        class Container {
            val names: List<String> = listOf()
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "List");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("List".to_string()));
}

#[test]
fn test_kotlin_type_generic_argument() {
    let support = KotlinSupport::new();
    let content = r#"
        class Container {
            val names: List<String> = listOf()
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("String".to_string()));
}

#[test]
fn test_kotlin_type_map() {
    let support = KotlinSupport::new();
    let content = r#"
        class Container {
            val scores: Map<String, Int> = mapOf()
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "Map");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("Map".to_string()));
}

#[test]
fn test_kotlin_type_parameter() {
    let support = KotlinSupport::new();
    let content = r#"
        class Container {
            fun process(input: List<Int>): Map<String, List<Int>> {
                return mapOf("results" to input)
            }
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "List<Int>");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("List".to_string()));
}

#[test]
fn test_kotlin_type_class_declaration() {
    let support = KotlinSupport::new();
    let content = r#"
        class MyClass {}
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("MyClass".to_string()));
}

#[test]
fn test_kotlin_type_interface_declaration() {
    let support = KotlinSupport::new();
    let content = r#"
        interface MyInterface {}
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "MyInterface");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("MyInterface".to_string()));
}

#[test]
fn test_kotlin_type_not_at_type_position() {
    let support = KotlinSupport::new();
    let content = r#"
        class Container {
            val names: List<String> = listOf()
            fun test() {
                names;
            }
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "names;");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, None);
}
