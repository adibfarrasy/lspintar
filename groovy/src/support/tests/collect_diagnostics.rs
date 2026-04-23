#![allow(unused_imports)]

use crate::GroovySupport;
use lsp_core::language_support::{ClassDeclarationData, LanguageSupport};
use tower_lsp::lsp_types::DiagnosticSeverity;

use super::*;

fn diagnostics_for(source: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    let support = GroovySupport::new();
    let (tree, content) = support.parse_str(source).expect("parse failed");
    support.collect_diagnostics(&tree, &content)
}

fn type_refs_for(source: &str) -> Vec<String> {
    let support = GroovySupport::new();
    let (tree, content) = support.parse_str(source).expect("parse failed");
    support
        .get_type_references(&tree, &content)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn declared_types_for(source: &str) -> Vec<String> {
    let support = GroovySupport::new();
    let (tree, content) = support.parse_str(source).expect("parse failed");
    support.get_declared_type_names(&tree, &content)
}

fn class_decls_for(source: &str) -> Vec<ClassDeclarationData> {
    let support = GroovySupport::new();
    let (tree, content) = support.parse_str(source).expect("parse failed");
    support.get_class_declarations(&tree, &content)
}

// --- duplicate_import ---

#[test]
fn test_no_duplicate_imports() {
    let source = r#"
import java.util.List
import java.util.Map
class Foo {}
"#;
    let diags = diagnostics_for(source);
    assert!(
        diags.iter().all(|d| d.code
            != Some(tower_lsp::lsp_types::NumberOrString::String(
                "duplicate_import".to_string()
            ))),
        "expected no duplicate_import diagnostic"
    );
}

#[test]
fn test_duplicate_import_emits_warning() {
    let source = r#"
import java.util.List
import java.util.List
class Foo {}
"#;
    let diags = diagnostics_for(source);
    let dup: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "duplicate_import".to_string(),
                ))
        })
        .collect();
    assert_eq!(dup.len(), 1, "expected exactly one duplicate_import diagnostic");
    assert_eq!(dup[0].severity, Some(DiagnosticSeverity::WARNING));
    assert!(dup[0].message.contains("java.util.List"));
}

#[test]
fn test_wildcard_import_duplicate_is_flagged() {
    let source = r#"
import java.util.*
import java.util.*
class Foo {}
"#;
    let diags = diagnostics_for(source);
    let dup: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "duplicate_import".to_string(),
                ))
        })
        .collect();
    assert_eq!(dup.len(), 1);
}

// --- unused_import ---

#[test]
fn test_used_import_not_flagged() {
    let source = r#"
import java.util.List
class Foo {
    List items
}
"#;
    let diags = diagnostics_for(source);
    assert!(
        diags.iter().all(|d| d.code
            != Some(tower_lsp::lsp_types::NumberOrString::String(
                "unused_import".to_string()
            ))),
        "expected no unused_import diagnostic for a used import"
    );
}

#[test]
fn test_unused_import_emits_warning() {
    let source = r#"
import java.util.List
import java.util.Map
class Foo {
    List items
}
"#;
    let diags = diagnostics_for(source);
    let unused: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "unused_import".to_string(),
                ))
        })
        .collect();
    assert_eq!(unused.len(), 1, "expected exactly one unused_import diagnostic");
    assert_eq!(unused[0].severity, Some(DiagnosticSeverity::WARNING));
    assert!(unused[0].message.contains("java.util.Map"));
}

#[test]
fn test_wildcard_import_never_flagged_as_unused() {
    let source = r#"
import java.util.*
class Foo {}
"#;
    let diags = diagnostics_for(source);
    assert!(
        diags.iter().all(|d| d.code
            != Some(tower_lsp::lsp_types::NumberOrString::String(
                "unused_import".to_string()
            ))),
        "wildcard imports should never be flagged as unused"
    );
}

// --- get_type_references ---

#[test]
fn test_type_refs_captures_field_type() {
    let source = r#"
class Foo {
    Bar field
}
"#;
    let refs = type_refs_for(source);
    assert!(refs.contains(&"Bar".to_string()), "expected 'Bar' in type refs, got: {refs:?}");
}

#[test]
fn test_type_refs_captures_parameter_type() {
    let source = r#"
class Foo {
    void doThing(Baz b) {}
}
"#;
    let refs = type_refs_for(source);
    assert!(refs.contains(&"Baz".to_string()), "expected 'Baz' in type refs, got: {refs:?}");
}

#[test]
fn test_type_refs_captures_superclass() {
    let source = r#"
class Child extends Parent {}
"#;
    let refs = type_refs_for(source);
    assert!(
        refs.contains(&"Parent".to_string()),
        "expected 'Parent' in type refs, got: {refs:?}"
    );
}

#[test]
fn test_type_refs_does_not_include_class_declaration_name() {
    let source = r#"
class MyClass {}
"#;
    let refs = type_refs_for(source);
    assert!(
        !refs.contains(&"MyClass".to_string()),
        "class declaration name should not appear as a type reference"
    );
}

// --- get_declared_type_names ---

#[test]
fn test_declared_types_captures_class_interface_enum() {
    let source = r#"
class Alpha {}
interface Beta {}
enum Gamma { A, B }
"#;
    let declared = declared_types_for(source);
    assert!(declared.contains(&"Alpha".to_string()));
    assert!(declared.contains(&"Beta".to_string()));
    assert!(declared.contains(&"Gamma".to_string()));
}

#[test]
fn test_declared_types_does_not_include_methods() {
    let source = r#"
class Foo {
    void doThing() {}
}
"#;
    let declared = declared_types_for(source);
    assert!(
        !declared.contains(&"doThing".to_string()),
        "method names should not appear in declared types"
    );
}

// --- get_class_declarations ---

#[test]
fn test_class_decl_captures_name_and_parents() {
    let source = r#"
class Child extends Parent implements Runnable, Serializable {}
"#;
    let decls = class_decls_for(source);
    assert_eq!(decls.len(), 1);
    let d = &decls[0];
    assert_eq!(d.name, "Child");
    assert!(!d.is_abstract);
    assert!(d.parents.contains(&"Parent".to_string()));
    assert!(d.parents.contains(&"Runnable".to_string()));
    assert!(d.parents.contains(&"Serializable".to_string()));
}

#[test]
fn test_abstract_class_flagged() {
    let source = r#"
abstract class Base {
    abstract void doThing()
}
"#;
    let decls = class_decls_for(source);
    assert_eq!(decls.len(), 1);
    assert!(decls[0].is_abstract);
}

#[test]
fn test_defined_method_names_captured() {
    let source = r#"
class Foo {
    void alpha() {}
    int beta() { return 1 }
}
"#;
    let decls = class_decls_for(source);
    assert_eq!(decls.len(), 1);
    let names: Vec<&str> = decls[0]
        .defined_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert!(names.contains(&"alpha"), "expected 'alpha' in defined methods");
    assert!(names.contains(&"beta"), "expected 'beta' in defined methods");
}

#[test]
fn test_inner_class_methods_not_counted_in_outer() {
    let source = r#"
class Outer {
    void outerMethod() {}
    class Inner {
        void innerMethod() {}
    }
}
"#;
    let decls = class_decls_for(source);
    let outer = decls.iter().find(|d| d.name == "Outer").expect("Outer not found");
    let outer_names: Vec<&str> = outer.defined_methods.iter().map(|m| m.name.as_str()).collect();
    assert!(outer_names.contains(&"outerMethod"));
    assert!(
        !outer_names.contains(&"innerMethod"),
        "inner class methods should not appear in outer class defined methods"
    );
}

#[test]
fn test_defined_method_param_types_distinguish_overloads() {
    let source = r#"
class Foo {
    void foo(String s) {}
    void foo(int n) {}
}
"#;
    let decls = class_decls_for(source);
    assert_eq!(decls.len(), 1);
    let sigs: Vec<(String, Vec<String>)> = decls[0]
        .defined_methods
        .iter()
        .map(|m| (m.name.clone(), m.param_types.clone()))
        .collect();
    assert!(sigs.contains(&("foo".to_string(), vec!["String".to_string()])));
    assert!(sigs.contains(&("foo".to_string(), vec!["int".to_string()])));
}

#[test]
fn test_interface_not_captured_as_class_declaration() {
    let source = r#"
interface MyInterface {
    void doThing()
}
"#;
    let decls = class_decls_for(source);
    assert!(decls.is_empty(), "interfaces should not appear as class declarations");
}

// --- unchecked_cast ---

fn unchecked_cast_diags_for(source: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    diagnostics_for(source)
        .into_iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "unchecked_cast".to_string(),
                ))
        })
        .collect()
}

#[test]
fn test_plain_cast_not_flagged() {
    let source = r#"
class Foo {
    void test(Object obj) {
        String s = (String) obj
    }
}
"#;
    assert!(
        unchecked_cast_diags_for(source).is_empty(),
        "plain cast to non-generic type should not be flagged"
    );
}

#[test]
fn test_generic_cast_flagged() {
    let source = r#"
class Foo {
    void test(Object obj) {
        List<String> items = (List<String>) obj
    }
}
"#;
    let diags = unchecked_cast_diags_for(source);
    assert_eq!(diags.len(), 1, "cast to generic type should be flagged");
    assert_eq!(diags[0].severity, Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING));
    assert!(diags[0].message.contains("List<String>"));
}

// --- duplicate_method_signature ---

fn dup_sig_diags_for(source: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    diagnostics_for(source)
        .into_iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "duplicate_method_signature".to_string(),
                ))
        })
        .collect()
}

#[test]
fn test_no_dup_sig_for_overloads() {
    let source = r#"
class Foo {
    void process(String x) {}
    void process(int x) {}
}
"#;
    assert!(
        dup_sig_diags_for(source).is_empty(),
        "methods with different param types should not be flagged"
    );
}

#[test]
fn test_dup_sig_same_name_same_params() {
    let source = r#"
class Foo {
    void process(String x) {}
    void process(String y) {}
}
"#;
    let diags = dup_sig_diags_for(source);
    assert_eq!(diags.len(), 1, "expected one duplicate_method_signature diagnostic");
    assert_eq!(diags[0].severity, Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR));
    assert!(diags[0].message.contains("process"));
}

#[test]
fn test_dup_sig_no_params() {
    let source = r#"
class Foo {
    void run() {}
    void run() {}
}
"#;
    let diags = dup_sig_diags_for(source);
    assert_eq!(diags.len(), 1, "expected one duplicate for zero-param methods");
}

#[test]
fn test_dup_sig_inner_class_does_not_affect_outer() {
    let source = r#"
class Outer {
    void run() {}
    class Inner {
        void run() {}
    }
}
"#;
    assert!(
        dup_sig_diags_for(source).is_empty(),
        "same method in inner class should not trigger duplicate on outer"
    );
}
