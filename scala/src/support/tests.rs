use lsp_core::{language_support::LanguageSupport, languages::Language, node_kind::NodeKind};

use crate::ScalaSupport;

fn support() -> ScalaSupport {
    ScalaSupport::new()
}

fn parse(src: &str) -> (tree_sitter::Tree, String) {
    support().parse_str(src).expect("scala parser available")
}

#[test]
fn get_language_returns_scala() {
    assert_eq!(support().get_language(), Language::Scala);
}

#[test]
fn parses_minimal_object() {
    let src = r#"
package com.example

object Hello {
  def main(args: Array[String]): Unit = println("hi")
}
"#;
    let (tree, _) = parse(src);
    assert!(!tree.root_node().has_error(), "tree-sitter-scala failed to parse minimal source");
}

#[test]
fn extracts_package_name() {
    let src = "package com.example.foo\n\nclass A\n";
    let (tree, content) = parse(src);
    assert_eq!(
        support().get_package_name(&tree, &content).as_deref(),
        Some("com.example.foo"),
    );
}

#[test]
fn extracts_explicit_imports_and_appends_implicit() {
    let src = r#"
package com.example

import scala.collection.mutable.ArrayBuffer
import java.util.{List, Map}

class A
"#;
    let (tree, content) = parse(src);
    let imports = support().get_imports(&tree, &content);
    assert!(
        imports.iter().any(|i| i.contains("scala.collection.mutable.ArrayBuffer")),
        "missing explicit single import: {imports:?}",
    );
    assert!(
        imports.iter().any(|i| i.contains("java.util")),
        "missing explicit grouped import: {imports:?}",
    );
    assert!(
        imports.iter().any(|i| i == "scala.Predef.*"),
        "implicit Predef import not appended: {imports:?}",
    );
    assert!(
        imports.iter().any(|i| i == "java.lang.*"),
        "implicit java.lang import not appended: {imports:?}",
    );
}

#[test]
fn classifies_top_level_nodes() {
    let src = r#"
package com.example

class C
object O
trait T
def free: Int = 1
val v: Int = 1
"#;
    let (tree, content) = parse(src);
    let support = support();
    let root = tree.root_node();
    let mut found_class = false;
    let mut found_object = false;
    let mut found_trait = false;
    let mut found_def = false;
    let mut found_val = false;

    for child in root.children(&mut root.walk()) {
        let Some(kind) = support.get_kind(&child) else {
            continue;
        };
        let name = support.get_short_name(&child, &content);
        match (kind, name.as_deref()) {
            (NodeKind::Class, Some("C")) => found_class = true,
            (NodeKind::Class, Some("O")) => found_object = true, // object treated as Class
            (NodeKind::Interface, Some("T")) => found_trait = true,
            (NodeKind::Function, Some("free")) => found_def = true,
            (NodeKind::Field, Some("v")) => found_val = true,
            _ => {}
        }
    }
    assert!(found_class, "class C not classified");
    assert!(found_object, "object O not classified");
    assert!(found_trait, "trait T not classified");
    assert!(found_def, "top-level def not classified");
    assert!(found_val, "top-level val not classified");
}

#[test]
fn enum_definition_classified_as_enum() {
    // Scala 3 enum
    let src = "package p\nenum Color { case Red, Green, Blue }\n";
    let (tree, content) = parse(src);
    let support = support();
    let mut found = false;
    let root = tree.root_node();
    for child in root.children(&mut root.walk()) {
        if support.get_kind(&child) == Some(NodeKind::Enum)
            && support.get_short_name(&child, &content).as_deref() == Some("Color")
        {
            found = true;
        }
    }
    assert!(found, "scala 3 enum not classified");
}

#[test]
fn collect_diagnostics_flags_syntax_error() {
    let src = "package p\nclass A {\n  def bad(\n}\n";
    let (tree, content) = parse(src);
    let diags = support().collect_diagnostics(&tree, &content);
    assert!(!diags.is_empty(), "expected at least one syntax error diagnostic");
}

#[test]
fn collect_diagnostics_clean_for_valid_source() {
    let src = "package p\n\nclass A { def m(x: Int): Int = x + 1 }\n";
    let (tree, content) = parse(src);
    let diags = support().collect_diagnostics(&tree, &content);
    assert!(diags.is_empty(), "unexpected diagnostics on valid source: {diags:?}");
}

#[test]
fn reserved_keywords_blocked_in_is_valid_identifier() {
    let s = support();
    assert!(!s.is_valid_identifier("class"));
    assert!(!s.is_valid_identifier("given"));
    assert!(!s.is_valid_identifier("using"));
    assert!(s.is_valid_identifier("foo"));
    assert!(s.is_valid_identifier("_under"));
}
