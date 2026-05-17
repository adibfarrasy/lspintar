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

// ---------- Phase 2: hierarchy + metadata ----------

fn first_child_with_kind<'a>(
    parent: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut c = parent.walk();
    parent.children(&mut c).find(|n| n.kind() == kind)
}

#[test]
fn get_extends_returns_single_parent() {
    let src = "class C extends Base";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    assert_eq!(support().get_extends(&cls, &content).as_deref(), Some("Base"));
}

#[test]
fn get_implements_returns_with_mixins_only() {
    let src = "class C extends Base with M1 with M2";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    assert_eq!(support().get_extends(&cls, &content).as_deref(), Some("Base"));
    assert_eq!(support().get_implements(&cls, &content), vec!["M1", "M2"]);
}

#[test]
fn get_implements_for_trait_extends_with() {
    let src = "trait T extends A with B";
    let (tree, content) = parse(src);
    let t = first_child_with_kind(tree.root_node(), "trait_definition").unwrap();
    assert_eq!(support().get_extends(&t, &content).as_deref(), Some("A"));
    assert_eq!(support().get_implements(&t, &content), vec!["B"]);
}

#[test]
fn get_extends_none_when_absent() {
    let src = "class C { val x = 1 }";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    assert!(support().get_extends(&cls, &content).is_none());
    assert!(support().get_implements(&cls, &content).is_empty());
}

#[test]
fn get_modifiers_captures_modifiers_block() {
    let src = "final private class C";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let mods = support().get_modifiers(&cls, &content);
    assert!(mods.iter().any(|m| m == "final"), "missing final in {mods:?}");
    assert!(mods.iter().any(|m| m.contains("private")), "missing private in {mods:?}");
}

#[test]
fn get_modifiers_captures_case_keyword() {
    let src = "case class CC(a: Int)";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let mods = support().get_modifiers(&cls, &content);
    assert!(mods.iter().any(|m| m == "case"), "case modifier not captured: {mods:?}");
}

#[test]
fn get_modifiers_captures_override_on_function() {
    let src = "class C extends A { override def f: Int = 1 }";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let body = first_child_with_kind(cls, "template_body").unwrap();
    let func = first_child_with_kind(body, "function_definition").unwrap();
    let mods = support().get_modifiers(&func, &content);
    assert!(mods.iter().any(|m| m == "override"), "override missing in {mods:?}");
}

#[test]
fn get_annotations_returns_full_text() {
    let src = "@Deprecated(\"x\")\n@Override\nclass Foo";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let anns = support().get_annotations(&cls, &content);
    assert!(anns.iter().any(|a| a.contains("Deprecated")), "{anns:?}");
    assert!(anns.iter().any(|a| a.contains("Override")), "{anns:?}");
}

#[test]
fn get_documentation_picks_up_scaladoc_previous_sibling() {
    let src = "/** doc */\nclass D";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let doc = support().get_documentation(&cls, &content);
    assert_eq!(doc.as_deref(), Some("/** doc */"));
}

#[test]
fn get_documentation_none_for_regular_block_comment() {
    let src = "/* not scaladoc */\nclass D";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    assert!(support().get_documentation(&cls, &content).is_none());
}

#[test]
fn get_parameters_for_function_returns_name_type_default() {
    let src = "def f(a: Int, b: String = \"x\"): Int = 0";
    let (tree, content) = parse(src);
    let func = first_child_with_kind(tree.root_node(), "function_definition").unwrap();
    let params = support().get_parameters(&func, &content).expect("params");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "a");
    assert_eq!(params[0].1.as_deref(), Some("Int"));
    assert!(params[0].2.is_none());
    assert_eq!(params[1].0, "b");
    assert_eq!(params[1].1.as_deref(), Some("String"));
    assert_eq!(params[1].2.as_deref(), Some("x"));
}

#[test]
fn get_parameters_for_class_constructor() {
    let src = "class P(val x: Int, var y: String = \"z\")";
    let (tree, content) = parse(src);
    let cls = first_child_with_kind(tree.root_node(), "class_definition").unwrap();
    let params = support().get_parameters(&cls, &content).expect("params");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "x");
    assert_eq!(params[0].1.as_deref(), Some("Int"));
    assert_eq!(params[1].0, "y");
    assert_eq!(params[1].2.as_deref(), Some("z"));
}

#[test]
fn get_parameters_generic_type() {
    let src = "def f(c: List[Int]): Unit = ()";
    let (tree, content) = parse(src);
    let func = first_child_with_kind(tree.root_node(), "function_definition").unwrap();
    let params = support().get_parameters(&func, &content).expect("params");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].1.as_deref(), Some("List[Int]"));
}

#[test]
fn get_return_for_function_picks_return_type_field() {
    let src = "def f(): Map[String, Int] = ???";
    let (tree, content) = parse(src);
    let func = first_child_with_kind(tree.root_node(), "function_definition").unwrap();
    let r = support().get_return(&func, &content);
    assert_eq!(r.as_deref(), Some("Map[String, Int]"));
}

#[test]
fn get_return_none_for_def_without_explicit_type() {
    let src = "def f() = 1";
    let (tree, content) = parse(src);
    let func = first_child_with_kind(tree.root_node(), "function_definition").unwrap();
    assert!(support().get_return(&func, &content).is_none());
}

#[test]
fn get_return_for_val_declaration_picks_type_field() {
    // Abstract val in a trait: `val x: Int`
    let src = "trait T { val x: Int }";
    let (tree, content) = parse(src);
    let t = first_child_with_kind(tree.root_node(), "trait_definition").unwrap();
    let body = first_child_with_kind(t, "template_body").unwrap();
    let v = first_child_with_kind(body, "val_declaration").unwrap();
    let r = support().get_return(&v, &content);
    assert_eq!(r.as_deref(), Some("Int"));
}

// ---------- Phase 3: position / type resolution ----------

use tower_lsp::lsp_types::Position;

/// Locate `needle` in `src` and return the Position of its first character.
fn pos_of(src: &str, needle: &str) -> Position {
    let byte = src.find(needle).unwrap_or_else(|| panic!("needle {needle:?} not found"));
    let prefix = &src[..byte];
    let line = prefix.matches('\n').count() as u32;
    let last_nl = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = (byte - last_nl) as u32;
    Position { line, character: col }
}

#[test]
fn get_literal_type_basic_kinds() {
    let src = "object O { val a = 1; val b = 1.0; val c = true; val d = \"hi\"; val e = 'x' }";
    let (tree, content) = parse(src);
    let s = support();
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1;")).as_deref(), Some("Int"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1.0")).as_deref(), Some("Double"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "true")).as_deref(), Some("Boolean"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "\"hi\"")).as_deref(), Some("String"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "'x'")).as_deref(), Some("Char"));
}

#[test]
fn find_ident_at_position_returns_simple_name() {
    let src = "object O { def use = foo }";
    let (tree, content) = parse(src);
    let id = support().find_ident_at_position(&tree, &content, &pos_of(src, "foo"));
    assert_eq!(id.as_ref().map(|(n, _)| n.as_str()), Some("foo"));
    assert!(id.unwrap().1.is_none());
}

#[test]
fn find_ident_at_position_returns_qualifier_for_field_access() {
    let src = "object O { def use = obj.method }";
    let (tree, content) = parse(src);
    let id = support().find_ident_at_position(&tree, &content, &pos_of(src, "method"));
    let (name, qual) = id.expect("ident");
    assert_eq!(name, "method");
    assert_eq!(qual.as_deref(), Some("obj"));
}

#[test]
fn find_variable_declaration_locates_val_in_same_scope() {
    let src = "object O { def use = { val x: Int = 1; x + 2 } }";
    let (tree, content) = parse(src);
    let decl = support().find_variable_declaration(&tree, &content, "x", &pos_of(src, "x +"));
    let (ty, p) = decl.expect("decl");
    assert_eq!(ty.as_deref(), Some("Int"));
    // The decl position is the `x` of `val x: Int = 1`, not the use site.
    assert!(p.character < pos_of(src, "x +").character);
}

#[test]
fn find_variable_declaration_locates_parameter() {
    let src = "object O { def use(arg: String): Int = { println(arg); 1 } }";
    let (tree, content) = parse(src);
    let decl = support().find_variable_declaration(&tree, &content, "arg", &pos_of(src, "println"));
    let (ty, _) = decl.expect("param decl");
    assert_eq!(ty.as_deref(), Some("String"));
}

#[test]
fn find_variable_declaration_locates_class_parameter() {
    let src = "class C(val x: Int) { def use = x + 1 }";
    let (tree, content) = parse(src);
    let decl = support().find_variable_declaration(&tree, &content, "x", &pos_of(src, "x + 1"));
    let (ty, _) = decl.expect("class param decl");
    assert_eq!(ty.as_deref(), Some("Int"));
}

#[test]
fn find_variable_type_returns_declared_type() {
    let src = "object O { val name: String = \"a\"; def use = name }";
    let (tree, content) = parse(src);
    let ty = support().find_variable_type(&tree, &content, "name", &pos_of(src, "= name"));
    assert_eq!(ty.as_deref(), Some("String"));
}

#[test]
fn find_declarations_in_scope_includes_local_and_param() {
    let src = "object O { def use(p: Int): Unit = { val q: String = \"a\"; () } }";
    let (tree, content) = parse(src);
    let decls = support().find_declarations_in_scope(&tree, &content, &pos_of(src, "() }"));
    let names: Vec<&str> = decls.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"p"), "param p missing: {names:?}");
    assert!(names.contains(&"q"), "local q missing: {names:?}");
    let q_ty = decls.iter().find(|(n, _)| n == "q").and_then(|(_, t)| t.clone());
    assert_eq!(q_ty.as_deref(), Some("String"));
}

#[test]
fn extract_call_arguments_returns_each_arg_with_position() {
    let src = "object O { def use = foo(1, \"x\", bar) }";
    let (tree, content) = parse(src);
    let args = support().extract_call_arguments(&tree, &content, &pos_of(src, "foo"));
    let args = args.expect("args");
    let texts: Vec<&str> = args.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(texts, vec!["1", "\"x\"", "bar"]);
}

#[test]
fn extract_call_arguments_handles_zero_args() {
    let src = "object O { def use = foo() }";
    let (tree, content) = parse(src);
    let args = support().extract_call_arguments(&tree, &content, &pos_of(src, "foo"));
    assert_eq!(args, Some(vec![]));
}

#[test]
fn get_method_receiver_and_params_extracts_receiver_chain() {
    let src = "object O { def use = obj.method(1, 2) }";
    let (tree, content) = parse(src);
    let root = tree.root_node();
    let node = lsp_core::ts_helper::get_node_at_position(&tree, &content, &pos_of(src, "method"))
        .unwrap_or(root);
    let res = support().get_method_receiver_and_params(node, &content, &pos_of(src, "method"));
    let (receiver, params) = res.expect("call site");
    assert_eq!(receiver, "obj");
    assert_eq!(params, vec!["1", "2"]);
}

#[test]
fn get_type_at_position_resolves_local_val() {
    let src = "object O { def use = { val x: String = \"a\"; x } }";
    let (tree, content) = parse(src);
    // Cursor sits on the trailing use-site `x` (after the val declaration).
    let p = pos_of(src, "x } }");
    let node = lsp_core::ts_helper::get_node_at_position(&tree, &content, &p).expect("node");
    let ty = support().get_type_at_position(node, &content, &p);
    assert_eq!(ty.as_deref(), Some("String"));
}

#[test]
fn get_type_at_position_resolves_literal() {
    let src = "object O { def use = 42 }";
    let (tree, content) = parse(src);
    let p = pos_of(src, "42");
    let node = lsp_core::ts_helper::get_node_at_position(&tree, &content, &p).expect("node");
    assert_eq!(support().get_type_at_position(node, &content, &p).as_deref(), Some("Int"));
}

// ---------- Phase 4: diagnostics data ----------

#[test]
fn get_type_references_collects_all_type_identifiers() {
    let src = "class C extends A with B { val x: Int = 1; def f(p: String): Long = 0L }";
    let (tree, content) = parse(src);
    let refs = support().get_type_references(&tree, &content);
    let names: Vec<String> = refs.iter().map(|(n, _)| n.clone()).collect();
    for needed in ["A", "B", "Int", "String", "Long"] {
        assert!(names.iter().any(|n| n == needed), "missing {needed}: {names:?}");
    }
}

#[test]
fn get_declared_type_names_lists_top_level_types() {
    let src = "package p\nclass C\nobject O\ntrait T\nenum E { case X }\ntype Alias = Int";
    let (tree, content) = parse(src);
    let mut names = support().get_declared_type_names(&tree, &content);
    names.sort();
    assert_eq!(names, vec!["Alias", "C", "E", "O", "T"]);
}

#[test]
fn get_class_declarations_marks_abstract_class_and_trait() {
    let src = "abstract class A { def todo: Int }\ntrait T { def m: Int }\nclass C extends A with T { def m = 1 }";
    let (tree, content) = parse(src);
    let decls = support().get_class_declarations(&tree, &content);
    let a = decls.iter().find(|d| d.name == "A").expect("A");
    assert!(a.is_abstract);
    assert!(a.parents.is_empty());
    let t = decls.iter().find(|d| d.name == "T").expect("T");
    assert!(t.is_abstract, "trait should be abstract");
    let c = decls.iter().find(|d| d.name == "C").expect("C");
    assert!(!c.is_abstract);
    assert_eq!(c.parents, vec!["A", "T"]);
    assert!(c.defined_methods.iter().any(|m| m.name == "m"));
}

#[test]
fn get_object_creations_extracts_new_sites() {
    let src = "object O { def use = { val a = new Foo(1); val b = new java.util.ArrayList[Int]() } }";
    let (tree, content) = parse(src);
    let news = support().get_object_creations(&tree, &content);
    let names: Vec<&str> = news.iter().map(|n| n.type_name.as_str()).collect();
    assert!(names.contains(&"Foo"), "Foo: {names:?}");
    assert!(names.contains(&"ArrayList"), "ArrayList (qualified): {names:?}");
}

#[test]
fn get_member_accesses_extracts_simple_receiver_calls() {
    let src = "object O { def use = { obj.method(1); other.bar() } }";
    let (tree, content) = parse(src);
    let accesses = support().get_member_accesses(&tree, &content);
    assert_eq!(accesses.len(), 2);
    let pairs: Vec<(String, String)> = accesses
        .iter()
        .map(|a| (a.receiver_name.clone(), a.member_name.clone()))
        .collect();
    assert!(pairs.contains(&("obj".to_string(), "method".to_string())));
    assert!(pairs.contains(&("other".to_string(), "bar".to_string())));
}

#[test]
fn get_member_accesses_skips_non_simple_receivers() {
    // Chained `a.b.c()` has a field_expression receiver — not a simple identifier.
    let src = "object O { def use = a.b.c() }";
    let (tree, content) = parse(src);
    let accesses = support().get_member_accesses(&tree, &content);
    assert!(
        accesses.is_empty(),
        "chained receiver should be skipped: {:?}",
        accesses.iter().map(|a| (&a.receiver_name, &a.member_name)).collect::<Vec<_>>()
    );
}

#[test]
fn get_generic_type_usages_counts_args() {
    let src = "class C { val xs: List[Int] = Nil; val m: Map[String, Int] = ???; val n: Either[A, B] = ??? }";
    let (tree, content) = parse(src);
    let usages = support().get_generic_type_usages(&tree, &content);
    let by_name: std::collections::HashMap<String, usize> =
        usages.iter().map(|u| (u.type_name.clone(), u.arg_count)).collect();
    assert_eq!(by_name.get("List"), Some(&1));
    assert_eq!(by_name.get("Map"), Some(&2));
    assert_eq!(by_name.get("Either"), Some(&2));
}

#[test]
fn get_override_methods_captures_override_def() {
    let src = "trait T { def f(x: Int): String }\nclass C extends T { override def f(x: Int): String = \"y\" }";
    let (tree, content) = parse(src);
    let overrides = support().get_override_methods(&tree, &content);
    assert_eq!(overrides.len(), 1);
    let o = &overrides[0];
    assert_eq!(o.containing_class, "C");
    assert_eq!(o.method_name, "f");
    assert_eq!(o.return_type.as_deref(), Some("String"));
}

#[test]
fn get_method_call_sites_records_args() {
    let src = "object O { def use(x: Int): Unit = { obj.foo(x, \"y\", 42); () } }";
    let (tree, content) = parse(src);
    let sites = support().get_method_call_sites(&tree, &content);
    let foo = sites.iter().find(|s| s.method_name == "foo").expect("foo call");
    assert_eq!(foo.receiver_name, "obj");
    assert_eq!(foo.args.len(), 3);
    assert_eq!(foo.args[0].node_kind, "identifier");
    assert_eq!(foo.args[0].text, "x");
    assert_eq!(foo.args[1].node_kind, "string");
    assert_eq!(foo.args[2].node_kind, "integer_literal");
}

// ---------- Phase 5 polish: enum cases ----------

#[test]
fn enum_cases_classified_as_field_with_names() {
    let src = "enum Color { case Red; case Green; case Blue }";
    let (tree, content) = parse(src);
    let s = support();
    let root = tree.root_node();
    let enum_node = first_child_with_kind(root, "enum_definition").expect("enum");
    let body = first_child_with_kind(enum_node, "enum_body").expect("enum_body");
    let mut case_names: Vec<String> = Vec::new();
    let mut c = body.walk();
    for child in body.children(&mut c) {
        if child.kind() != "enum_case_definitions" {
            continue;
        }
        let mut cc = child.walk();
        for cd in child.children(&mut cc) {
            if cd.kind() != "simple_enum_case" {
                continue;
            }
            assert_eq!(s.get_kind(&cd), Some(NodeKind::Field));
            if let Some(name) = s.get_short_name(&cd, &content) {
                case_names.push(name);
            }
        }
    }
    assert_eq!(case_names, vec!["Red", "Green", "Blue"]);
}

#[test]
fn enum_grouped_cases_each_indexed() {
    let src = "enum Direction { case North, South, East, West }";
    let (tree, content) = parse(src);
    let s = support();
    let root = tree.root_node();
    let enum_node = first_child_with_kind(root, "enum_definition").expect("enum");
    let body = first_child_with_kind(enum_node, "enum_body").expect("enum_body");
    let mut names: Vec<String> = Vec::new();
    let mut c = body.walk();
    for child in body.children(&mut c) {
        if child.kind() != "enum_case_definitions" {
            continue;
        }
        let mut cc = child.walk();
        for cd in child.children(&mut cc) {
            if cd.kind() == "simple_enum_case"
                && let Some(n) = s.get_short_name(&cd, &content) {
                    names.push(n);
                }
        }
    }
    assert_eq!(names, vec!["North", "South", "East", "West"]);
}

// ---------- Literal suffix refinement ----------

#[test]
fn literal_suffixes_refine_int_long_float_double() {
    let src = "object O { val a = 1; val b = 1L; val c = 1.0f; val d = 1.0; val e = 1.0d }";
    let (tree, content) = parse(src);
    let s = support();
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1;")).as_deref(), Some("Int"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1L")).as_deref(), Some("Long"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1.0f")).as_deref(), Some("Float"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1.0;")).as_deref(), Some("Double"));
    assert_eq!(s.get_literal_type(&tree, &content, &pos_of(src, "1.0d")).as_deref(), Some("Double"));
}

// ---------- find_local_references ----------

#[test]
fn find_local_references_collects_use_sites_for_val() {
    let src = "object O { def use = { val x: Int = 1; x + x + 2 } }";
    let (tree, content) = parse(src);
    let decl_pos = pos_of(src, "x: Int");
    let refs = support()
        .find_local_references(&tree, &content, &decl_pos)
        .expect("references");
    // 1 declaration + 2 use sites of `x` in `x + x` = 3 occurrences.
    assert_eq!(refs.len(), 3);
}

#[test]
fn find_local_references_for_parameter() {
    let src = "object O { def use(p: Int): Int = { val q = p * 2; p + q } }";
    let (tree, content) = parse(src);
    let decl_pos = pos_of(src, "p: Int");
    let refs = support()
        .find_local_references(&tree, &content, &decl_pos)
        .expect("references");
    // declaration + `p * 2` + `p + q` = 3 occurrences.
    assert_eq!(refs.len(), 3);
}

#[test]
fn find_local_references_excludes_shadowed_inner_binding() {
    let src = "object O { def use(x: Int): Int = { val r = x + 1; { val x: Int = 99; x + 1 }; x } }";
    let (tree, content) = parse(src);
    let decl_pos = pos_of(src, "x: Int");
    let refs = support()
        .find_local_references(&tree, &content, &decl_pos)
        .expect("references");
    // outer x is referenced at: declaration, `x + 1` (first), and the
    // trailing `x` after the shadowing block.  The inner `val x` and its
    // `x + 1` use are skipped.
    assert_eq!(refs.len(), 3);
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
