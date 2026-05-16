#![allow(unused_imports)]

use crate::{GroovySupport, constants::GROOVY_IMPLICIT_IMPORTS};
use lsp_core::language_support::LanguageSupport;

use super::*;

fn implicits_plus(extra: Vec<&str>) -> Vec<String> {
    GROOVY_IMPLICIT_IMPORTS
        .iter()
        .map(|s| s.to_string())
        .chain(extra.into_iter().map(String::from))
        .collect()
}

fn imports_of(content: &str) -> Vec<String> {
    let support = GroovySupport::new();
    let parsed = support.parse_str(content).expect("cannot parse content");
    support.get_imports(&parsed.0, &parsed.1)
}

#[test]
fn returns_implicits_plus_explicit_imports() {
    let content = "package com.example.app\n\nimport com.example.Foo\nimport java.lang.*";
    assert_eq!(
        imports_of(content),
        implicits_plus(vec!["com.example.Foo", "java.lang.*"])
    );
}

#[test]
fn file_without_explicit_imports_returns_just_implicits() {
    let content = "package com.example.app\n\nclass Foo {}";
    assert_eq!(imports_of(content), implicits_plus(vec![]));
}

#[test]
fn file_without_package_still_returns_implicits() {
    let content = "import com.example.Foo\n\nclass Foo {}";
    assert_eq!(imports_of(content), implicits_plus(vec!["com.example.Foo"]));
}

#[test]
fn static_import_is_recorded_with_static_prefix() {
    // The Groovy support keeps the literal `static ` prefix on static imports so
    // downstream resolution code can distinguish them from regular type imports.
    // Pinning this behaviour — if it changes, the downstream resolver almost
    // certainly needs updating too.
    let content = "package com.example\n\nimport static com.example.Helper.doThing";
    let imports = imports_of(content);
    assert!(
        imports.iter().any(|i| i == "static com.example.Helper.doThing"),
        "expected `static com.example.Helper.doThing`, got: {imports:?}"
    );
}

#[test]
fn aliased_import_uses_alias_target_path() {
    let content = "package com.example\n\nimport com.example.Foo as Bar";
    let imports = imports_of(content);
    // The aliased target's FQN must be recorded so symbol lookup still works.
    assert!(
        imports.iter().any(|i| i == "com.example.Foo"),
        "aliased import target should be recorded, got: {imports:?}"
    );
}

#[test]
fn multiple_imports_preserve_source_order() {
    let content = "package com.example\n\nimport com.a.A\nimport com.b.B\nimport com.c.C";
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
    assert!(positions[0] < positions[1] && positions[1] < positions[2],
        "imports should retain declaration order, got positions: {positions:?}");
}
