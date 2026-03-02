#![allow(unused_imports)]

use crate::JavaSupport;
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_java_type_field_declaration() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            String name;
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("String".to_string()));
}

#[test]
fn test_java_type_variable_declaration() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Integer count = 0;
            }
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "Integer");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("Integer".to_string()));
}

#[test]
fn test_java_type_parameter() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test(String input) {}
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("String".to_string()));
}

#[test]
fn test_java_type_class_declaration() {
    let support = JavaSupport::new();
    let content = r#"
        class MyClass {}
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("MyClass".to_string()));
}

#[test]
fn test_java_type_interface_declaration() {
    let support = JavaSupport::new();
    let content = r#"
        interface MyInterface {}
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "MyInterface");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("MyInterface".to_string()));
}

#[test]
fn test_java_type_enum_declaration() {
    let support = JavaSupport::new();
    let content = r#"
        enum Status { ACTIVE, INACTIVE }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "Status");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("Status".to_string()));
}

#[test]
fn test_java_type_generic() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            List<String> items;
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("String".to_string()));

    let pos = find_position(content, "List");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("List".to_string()));
}

#[test]
fn test_java_type_array() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            String[] tags;
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, Some("String".to_string()));
}

#[test]
fn test_java_type_not_at_type_position() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Object bar = new Object();
                bar;
            }
        }
        "#;
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, "bar;");
    let type_ = support.get_type_at_position(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(type_, None);
}
