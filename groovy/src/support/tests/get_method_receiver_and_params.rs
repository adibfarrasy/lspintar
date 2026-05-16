#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

fn receiver_and_params(content: &str, method: &str) -> Option<(String, Vec<String>)> {
    let support = GroovySupport::new();
    let parsed = support.parse_str(content).expect("cannot parse content");
    let pos = find_position(content, method);
    support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos)
}

#[test]
fn interface_method_no_params() {
    let content = r#"
        interface Foo {
            void doSomething()
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "doSomething"),
        Some(("Foo".to_string(), vec![]))
    );
}

#[test]
fn class_method_no_params() {
    let content = r#"
        class Foo {
            void doSomething() {
                println "test"
            }
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "doSomething"),
        Some(("Foo".to_string(), vec![]))
    );
}

#[test]
fn typed_params_are_returned_in_order() {
    let content = r#"
        class Foo {
            String greet(String name, int age) {
                return "$name $age"
            }
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "greet"),
        Some((
            "Foo".to_string(),
            vec!["String".to_string(), "int".to_string()]
        ))
    );
}

#[test]
fn generic_param_types_preserve_type_arguments() {
    let content = r#"
        class Foo {
            void process(List<String> items, Map<Integer, User> users) {}
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "process"),
        Some((
            "Foo".to_string(),
            vec![
                "List<String>".to_string(),
                "Map<Integer, User>".to_string(),
            ]
        ))
    );
}

#[test]
fn nested_class_method_uses_inner_class_as_receiver() {
    let content = r#"
        class Outer {
            class Inner {
                void innerMethod() {}
            }
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "innerMethod"),
        Some(("Inner".to_string(), vec![]))
    );
}

#[test]
fn multiple_methods_resolved_independently() {
    let content = r#"
        class Foo {
            void first() {}
            void second(int x) {}
        }
    "#;
    assert_eq!(
        receiver_and_params(content, "first"),
        Some(("Foo".to_string(), vec![]))
    );
    assert_eq!(
        receiver_and_params(content, "second"),
        Some(("Foo".to_string(), vec!["int".to_string()]))
    );
}

#[test]
fn def_typed_parameter_recorded() {
    // Groovy-specific: `def` as a parameter type. Pin whatever the support
    // produces — explicit so we notice if the behaviour shifts.
    let content = r#"
        class Foo {
            void poly(def value) {}
        }
    "#;
    let result = receiver_and_params(content, "poly");
    let (receiver, params) = result.expect("expected Some");
    assert_eq!(receiver, "Foo");
    assert_eq!(params.len(), 1, "expected 1 parameter, got: {params:?}");
}

#[test]
fn trait_method_receiver_is_trait_name() {
    // Groovy traits behave like interfaces with default impls. The vendored
    // tree-sitter-groovy grammar was patched to add a `trait_declaration`
    // rule whose body reuses `class_body`, so trait methods are now extracted
    // through the same path as class methods.
    let content = r#"
        trait Greeter {
            String greet(String name) { return "hi $name" }
        }
    "#;
    let result = receiver_and_params(content, "greet");
    let (receiver, params) = result.expect("expected Some");
    assert_eq!(receiver, "Greeter");
    assert_eq!(params, vec!["String".to_string()]);
}
