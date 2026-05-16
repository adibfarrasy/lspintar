#![allow(unused_imports)]
use super::*;
use crate::{KotlinSupport, constants::KOTLIN_IMPLICIT_IMPORTS};
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};
use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

fn implicits_plus(extra: Vec<&str>) -> Vec<String> {
    KOTLIN_IMPLICIT_IMPORTS
        .iter()
        .map(|s| s.to_string())
        .chain(extra.into_iter().map(String::from))
        .collect()
}

fn imports_of(content: &str) -> Vec<String> {
    let support = KotlinSupport::new();
    let parsed = support.parse_str(content).expect("cannot parse content");
    support.get_imports(&parsed.0, &parsed.1)
}

#[test]
fn returns_implicits_plus_explicit_imports() {
    let content = "package com.example.app\n\nimport com.example.Foo\nimport java.util.*";
    assert_eq!(
        imports_of(content),
        implicits_plus(vec!["com.example.Foo", "java.util.*"])
    );
}

#[test]
fn file_without_explicit_imports_returns_just_implicits() {
    let content = "package com.example.app\n\nclass Foo";
    assert_eq!(imports_of(content), implicits_plus(vec![]));
}

#[test]
fn file_without_package_still_returns_implicits() {
    let content = "import com.example.Foo\n\nclass Foo";
    assert_eq!(imports_of(content), implicits_plus(vec!["com.example.Foo"]));
}

#[test]
fn aliased_import_strips_alias_and_keeps_target_fqn() {
    // `import com.foo.Bar as Baz` — downstream FQN-based lookup needs the
    // target path, not the local alias. We strip ` as <alias>` so the recorded
    // import string is a valid FQN.
    let content = "package com.example\n\nimport com.example.Foo as Bar";
    let imports = imports_of(content);
    assert!(
        imports.iter().any(|i| i == "com.example.Foo"),
        "expected aliased import target FQN, got: {imports:?}"
    );
    assert!(
        !imports.iter().any(|i| i.contains(" as ")),
        "no import should retain the ` as <alias>` suffix, got: {imports:?}"
    );
}

#[test]
fn multiple_imports_preserve_source_order() {
    let content = "package com.example\n\n\
        import com.a.A\n\
        import com.b.B\n\
        import com.c.C";
    let imports = imports_of(content);
    let positions: Vec<_> = ["com.a.A", "com.b.B", "com.c.C"]
        .iter()
        .map(|needle| {
            imports
                .iter()
                .position(|i| i == needle)
                .unwrap_or_else(|| panic!("missing {needle} in {imports:?}"))
        })
        .collect();
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "imports should retain declaration order, got positions: {positions:?}"
    );
}

#[test]
fn implicit_kotlin_imports_always_present() {
    // kotlin.* and similar implicit packages must appear regardless of what
    // the user writes — otherwise common types like Pair/List would not resolve.
    let imports = imports_of("package x\n\nimport com.foo.Bar\nclass C");
    assert!(
        !imports.is_empty(),
        "expected implicit imports plus user imports"
    );
    assert!(
        imports.iter().any(|i| i.starts_with("kotlin.")),
        "kotlin.* implicits must be present, got: {imports:?}"
    );
}
