#![allow(unused_imports)]

use crate::JavaSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_find_variable_type() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar();
                bar.doSomething();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar.");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "bar", &pos);
    assert_eq!(var_type, Some("Bar".to_string()));
}

#[test]
fn test_find_variable_type_with_generics() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<String> items = new ArrayList<>();
                items.add("test");
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "items.add");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "items", &pos);
    assert_eq!(var_type, Some("List<String>".to_string()));
}

#[test]
fn test_find_parameter_type() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test(User user) {
                user.getName();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user.getName");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "user", &pos);
    assert_eq!(var_type, Some("User".to_string()));
}

#[test]
fn test_find_field_type() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            private String name;
            void test() {
                name.toLowerCase();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name.toLowerCase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "name", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_find_this_type_nested_class() {
    let support = JavaSupport::new();
    let content = r#"
        class Outer {
            String outerField;
            class Inner {
                Integer innerField;
                void test() {
                    this.innerField.toString();
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "this.innerField");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "this", &pos);
    assert_eq!(var_type, Some("Inner".to_string()));
}
