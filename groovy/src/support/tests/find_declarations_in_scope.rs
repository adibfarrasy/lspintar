#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

#[test]
fn test_find_declarations_in_scope_local_vars() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar()
                String name = "test"
                bar.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar.doSomething");
    let decls = support.find_declarations_in_scope(&parsed.0, &parsed.1, &pos);
    assert!(
        decls
            .iter()
            .any(|(name, t)| name == "bar" && t.as_deref() == Some("Bar"))
    );
    assert!(
        decls
            .iter()
            .any(|(name, t)| name == "name" && t.as_deref() == Some("String"))
    );
}

#[test]
fn test_find_declarations_in_scope_excludes_after_cursor() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar()
                bar.doSomething()
                String name = "test"
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar.doSomething");
    let decls = support.find_declarations_in_scope(&parsed.0, &parsed.1, &pos);
    assert!(decls.iter().any(|(name, _)| name == "bar"));
    assert!(!decls.iter().any(|(name, _)| name == "name"));
}

#[test]
fn test_find_declarations_in_scope_includes_parameters() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test(User user, String input) {
                user.getName()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user.getName");
    let decls = support.find_declarations_in_scope(&parsed.0, &parsed.1, &pos);
    assert!(
        decls
            .iter()
            .any(|(name, t)| name == "user" && t.as_deref() == Some("User"))
    );
    assert!(
        decls
            .iter()
            .any(|(name, t)| name == "input" && t.as_deref() == Some("String"))
    );
}

#[test]
fn test_find_declarations_in_scope_includes_fields() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            private String name
            void test() {
                name.toLowerCase()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name.toLowerCase");
    let decls = support.find_declarations_in_scope(&parsed.0, &parsed.1, &pos);
    assert!(
        decls
            .iter()
            .any(|(n, t)| n == "name" && t.as_deref() == Some("String"))
    );
}
