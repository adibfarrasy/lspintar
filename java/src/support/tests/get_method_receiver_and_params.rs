#![allow(unused_imports)]

use crate::JavaSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_get_method_receiver_type_interface() {
    let support = JavaSupport::new();
    let content = r#"
        interface Foo {
            void doSomething();
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Foo".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_class() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void doSomething() {
                System.out.println("test");
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Foo".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_with_parameters() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void doSomething(String name, int age) {
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(
        receiver_type,
        Some((
            "Foo".to_string(),
            vec!["String".to_string(), "int".to_string()]
        ))
    );
}

#[test]
fn test_get_method_receiver_type_enum() {
    let support = JavaSupport::new();
    let content = r#"
        enum Color {
            RED, GREEN, BLUE;
            void display() {
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "display");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Color".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_with_generics() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void process(List<String> items, Map<Integer, User> users) {
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "process");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(
        receiver_type,
        Some((
            "Foo".to_string(),
            vec!["List<String>".to_string(), "Map<Integer, User>".to_string()]
        ))
    );
}

#[test]
fn test_get_method_receiver_type_nested_class() {
    let support = JavaSupport::new();
    let content = r#"
        class Outer {
            class Inner {
                void innerMethod() {
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "innerMethod");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Inner".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_multiple_methods() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void first() {}
            void second(int x) {}
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let pos = find_position(content, "first");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Foo".to_string(), vec![])));

    let pos = find_position(content, "second");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(
        receiver_type,
        Some(("Foo".to_string(), vec!["int".to_string()]))
    );
}
