#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_find_declarations_in_scope_local_vars() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar: Bar = Bar()
                val name: String = "test"
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
            .any(|(n, t)| n == "bar" && t.as_deref() == Some("Bar"))
    );
    assert!(
        decls
            .iter()
            .any(|(n, t)| n == "name" && t.as_deref() == Some("String"))
    );
}

#[test]
fn test_find_declarations_in_scope_excludes_after_cursor() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar: Bar = Bar()
                bar.doSomething()
                val name: String = "test"
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar.doSomething");
    let decls = support.find_declarations_in_scope(&parsed.0, &parsed.1, &pos);
    assert!(decls.iter().any(|(n, _)| n == "bar"));
    assert!(!decls.iter().any(|(n, _)| n == "name"));
}

#[test]
fn test_find_declarations_in_scope_includes_parameters() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test(user: User, input: String) {
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
            .any(|(n, t)| n == "user" && t.as_deref() == Some("User"))
    );
    assert!(
        decls
            .iter()
            .any(|(n, t)| n == "input" && t.as_deref() == Some("String"))
    );
}

#[test]
fn test_find_declarations_in_scope_includes_fields() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            val name: String = "test"
            fun test() {
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

#[test]
fn test_find_declarations_in_scope_inferred_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
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
            .any(|(n, t)| n == "bar" && t.as_deref() == Some("Bar"))
    );
}
