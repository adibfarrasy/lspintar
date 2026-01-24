#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_get_method_receiver_type_interface() {
    let support = KotlinSupport::new();
    let content = r#"
        interface Foo {
            fun doSomething()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun doSomething() {
                println("test")
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun doSomething(name: String, age: Int) {
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
            vec!["String".to_string(), "Int".to_string()]
        ))
    );
}

#[test]
fn test_get_method_receiver_type_enum() {
    let support = KotlinSupport::new();
    let content = r#"
        enum class Color {
            RED, GREEN, BLUE;
            fun display() {
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun process(items: List<String>, users: Map<Int, User>) {
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
            vec!["List<String>".to_string(), "Map<Int, User>".to_string()]
        ))
    );
}

#[test]
fn test_get_method_receiver_type_nested_class() {
    let support = KotlinSupport::new();
    let content = r#"
        class Outer {
            inner class Inner {
                fun innerMethod() {
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun first() {}
            fun second(x: Int) {}
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
        Some(("Foo".to_string(), vec!["Int".to_string()]))
    );
}

#[test]
fn test_get_method_receiver_type_object_declaration() {
    let support = KotlinSupport::new();
    let content = r#"
        object Singleton {
            fun doSomething() {
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Singleton".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_data_class() {
    let support = KotlinSupport::new();
    let content = r#"
        data class Person(val name: String, val age: Int) {
            fun greet() = "Hello, $name"
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "greet");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Person".to_string(), vec![])));
}

#[test]
fn test_get_method_receiver_type_with_nullable_types() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun process(name: String?, count: Int?) {
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
            vec!["String?".to_string(), "Int?".to_string()]
        ))
    );
}

#[test]
fn test_get_method_receiver_type_with_default_parameters() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun greet(name: String = "World") {
                println("Hello, $name")
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "greet");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(
        receiver_type,
        Some(("Foo".to_string(), vec!["String".to_string()]))
    );
}

#[test]
fn test_get_method_receiver_type_extension_function() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun String.customExtension() {
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "customExtension");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    // NOTE: For extension functions, the receiver type would be String, not Foo
    // But the containing class is still Foo
    assert_eq!(receiver_type, Some(("Foo".to_string(), vec![])));
}
