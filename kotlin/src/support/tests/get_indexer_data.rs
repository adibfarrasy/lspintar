#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_get_ident_range() {
    let support = KotlinSupport::new();
    let content = "class MyClass {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();

    let support = KotlinSupport::new();
    let range = support.get_ident_range(&class_node);

    assert_eq!(
        range,
        Some(Range {
            start: Position {
                line: 0u32,
                character: 6u32,
            },
            end: Position {
                line: 0u32,
                character: 13u32,
            },
        })
    );
}

#[test]
fn test_get_package_name() {
    let support = KotlinSupport::new();
    let content = "package com.example.app\n\nclass Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let node_name = support.get_package_name(&parsed.0, &parsed.1);
    assert_eq!(node_name, Some("com.example.app".to_string()));
}

#[test]
fn test_get_type() {
    let support = KotlinSupport::new();
    let content = "package com.example.app\n\nclass Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Class));

    let content = "package com.example.app\n\ninterface Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "interface_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Interface));

    let content = "package com.example.app\n\nenum class Color { RED, GREEN, BLUE }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Enum));

    let content = "class Foo { fun myFunction() { return } }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Function));

    let content = "package com.example.app\n\nclass Foo { val bar: String = \"\" }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "property_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Field));

    let content = "package com.example.app\n\nimport java.util.List\n\nclass Foo { val items: List<String> = listOf() }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "property_declaration").unwrap();
    let node_type = support.get_type(&node);
    assert_eq!(node_type, Some(NodeType::Field));
}

#[test]
fn test_get_short_name() {
    let support = KotlinSupport::new();

    let content = "package com.example.app\n\nclass Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let short_name = support.get_short_name(&node, &parsed.1);
    assert_eq!(short_name, Some("Foo".to_string()));

    let content = "package com.example.app\n\ninterface Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "interface_declaration").unwrap();
    let short_name = support.get_short_name(&node, &parsed.1);
    assert_eq!(short_name, Some("Foo".to_string()));

    let content = "package com.example.app\n\nenum class Color { RED, GREEN, BLUE }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let short_name = support.get_short_name(&node, &parsed.1);
    assert_eq!(short_name, Some("Color".to_string()));

    let content = "class Foo { fun myFunction() {} }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
    let short_name = support.get_short_name(&node, &parsed.1);
    assert_eq!(short_name, Some("myFunction".to_string()));

    let content = "package com.example.app\n\nclass Foo { val bar: String = \"\" }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "property_declaration").unwrap();
    let node_name = support.get_short_name(&node, &parsed.1);
    assert_eq!(node_name, Some("bar".to_string()));
}

#[test]
fn test_get_extends() {
    let support = KotlinSupport::new();
    let content = "package com.example.app\n\nclass Foo : Bar() {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let node = find_node_by_kind(parsed.0.root_node(), "delegation_specifier").unwrap();
    let node_name = support.get_extends(&node, &parsed.1);
    assert_eq!(node_name, Some("Bar".to_string()));
}

#[test]
fn test_get_implements() {
    let support = KotlinSupport::new();
    let content = "package com.example.app\n\nclass Foo : Bar, Baz {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let node_names = support.get_implements(&class_node, &parsed.1);
    assert_eq!(node_names, vec!["Bar", "Baz"]);
}

#[test]
fn test_get_modifiers() {
    let support = KotlinSupport::new();

    let content = "public abstract class Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let modifiers = support.get_modifiers(&node, &parsed.1);
    assert_eq!(modifiers, vec!["public", "abstract"]);

    let content = "class Bar { private fun test() {} }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let method_node = find_node_by_kind(class_node, "function_declaration").unwrap();
    let modifiers = support.get_modifiers(&method_node, &parsed.1);
    assert_eq!(modifiers, vec!["private"]);

    let content = "class Baz { val name: String = \"\" }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let field_node = find_node_by_kind(class_node, "property_declaration").unwrap();
    let modifiers = support.get_modifiers(&field_node, &parsed.1);
    assert!(modifiers.is_empty());

    let content = "class Qux {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let modifiers = support.get_modifiers(&node, &parsed.1);
    assert!(modifiers.is_empty());
}

#[test]
fn test_get_annotations() {
    let support = KotlinSupport::new();

    let content = "@Component\n@Service\nclass Foo {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let annotations = support.get_annotations(&node, &parsed.1);
    assert_eq!(annotations, vec!["Component", "Service"]);

    let content = "class Bar {\n    @Override\n    @Deprecated\n    fun test() {}\n}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let method_node = find_node_by_kind(class_node, "function_declaration").unwrap();
    let annotations = support.get_annotations(&method_node, &parsed.1);
    assert_eq!(annotations, vec!["Override", "Deprecated"]);

    let content = "class Baz {\n    @Autowired\n    val name: String = \"\"\n}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let field_node = find_node_by_kind(class_node, "property_declaration").unwrap();
    let annotations = support.get_annotations(&field_node, &parsed.1);
    assert_eq!(annotations, vec!["Autowired"]);

    let content = "class Qux {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let annotations = support.get_annotations(&node, &parsed.1);
    assert!(annotations.is_empty());
}

#[test]
fn test_get_documentation() {
    let support = KotlinSupport::new();

    let content = r#"
        /**
         * This is a test class
         * with multiple lines
         */
        class Foo {}
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let docs = support.get_documentation(&node, &parsed.1).unwrap();
    assert!(docs.contains("This is a test class"));

    let content = r#"
        class Bar {
            /**
             * Test method
             */
            fun test() {}
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let method_node = find_node_by_kind(class_node, "function_declaration").unwrap();
    let docs = support.get_documentation(&method_node, &parsed.1).unwrap();
    assert!(docs.contains("Test method"));

    let content = "class Baz {}";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
    let docs = support.get_documentation(&node, &parsed.1);
    assert!(docs.is_none());
}

#[test]
fn test_get_parameters() {
    let support = KotlinSupport::new();

    let content = "class Foo { fun myFunction(arg1: String, arg2: Int) {} }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
    let arguments = support.get_parameters(&node, &parsed.1).unwrap();
    assert_eq!(
        arguments,
        vec![
            ("arg1".to_string(), Some("String".to_string()), None),
            ("arg2".to_string(), Some("Int".to_string()), None),
        ]
    );
}

#[test]
fn test_get_return() {
    let support = KotlinSupport::new();

    let content = "class Bar { fun myFunction(arg1: String, arg2: Int): Foo { return Foo() } }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
    let ret = support.get_return(&node, &parsed.1);
    assert_eq!(ret, Some("Foo".to_string()));

    let content = "class Bar { fun myFunction(arg1: String, arg2: Int) {} }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
    let ret = support.get_return(&node, &parsed.1);
    assert_eq!(ret, None);

    let content = "class UserService { private val myVar: String = \"\" }";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node = find_node_by_kind(parsed.0.root_node(), "property_declaration").unwrap();
    let ret = support.get_return(&node, &parsed.1);
    assert_eq!(ret, Some("String".to_string()));
}
