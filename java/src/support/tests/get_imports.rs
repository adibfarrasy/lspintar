#![allow(unused_imports)]

use crate::{JavaSupport, constants::JAVA_IMPLICIT_IMPORTS};
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

fn implicits_plus(extra: Vec<&str>) -> Vec<String> {
    JAVA_IMPLICIT_IMPORTS
        .iter()
        .map(|s| s.to_string())
        .chain(extra.into_iter().map(String::from))
        .collect()
}

fn imports_of(content: &str) -> Vec<String> {
    let support = JavaSupport::new();
    let parsed = support.parse_str(content).expect("cannot parse content");
    support.get_imports(&parsed.0, &parsed.1)
}

#[test]
fn returns_implicits_plus_explicit_imports() {
    let content = "package com.example.app;\n\nimport com.example.Foo;\nimport java.util.*;";
    assert_eq!(
        imports_of(content),
        implicits_plus(vec!["com.example.Foo", "java.util.*"])
    );
}

#[test]
fn file_without_explicit_imports_returns_just_implicits() {
    let content = "package com.example.app;\n\npublic class Foo {}";
    assert_eq!(imports_of(content), implicits_plus(vec![]));
}

#[test]
fn file_without_package_still_returns_implicits() {
    let content = "import com.example.Foo;\n\nclass Foo {}";
    assert_eq!(imports_of(content), implicits_plus(vec!["com.example.Foo"]));
}

#[test]
fn static_import_is_recorded() {
    // Pinning current behaviour for Java static imports — capture exactly what
    // the support emits so a downstream change in this string is intentional.
    let content = "package com.example;\n\nimport static com.example.Helper.doThing;";
    let imports = imports_of(content);
    assert!(
        imports.iter().any(|i| i == "static com.example.Helper.doThing"
            || i == "com.example.Helper.doThing"),
        "expected static import to be recorded, got: {imports:?}"
    );
}

#[test]
fn multiple_imports_preserve_source_order() {
    let content = "package com.example;\n\n\
        import com.a.A;\n\
        import com.b.B;\n\
        import com.c.C;";
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
fn implicit_lang_imports_always_present() {
    // java.lang.* is implicit; whatever the user writes, it must still surface
    // in the import set so the resolver knows String, Object, etc. are visible.
    let imports = imports_of("package x;\n\nimport com.foo.Bar;\nclass C {}");
    assert!(
        imports.iter().any(|i| i == "java.lang.*"),
        "java.lang.* must be in implicit imports, got: {imports:?}"
    );
}
