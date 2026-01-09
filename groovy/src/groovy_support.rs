use lsp_core::{
    language_support::{LanguageSupport, ParameterResult, ParseResult},
    languages::Language,
    node_types::NodeType,
    ts_helper,
};
use std::{fs, path::Path, sync::Mutex};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Tree};

pub struct GroovySupport {
    parser: Mutex<Parser>,
}

impl GroovySupport {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_groovy::language())
            .unwrap();
        Self {
            parser: Mutex::new(parser),
        }
    }
}

impl LanguageSupport for GroovySupport {
    fn get_language(&self) -> Language {
        Language::Groovy
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_groovy::language()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        self.parser
            .try_lock()
            .expect("failed to get parser")
            .parse(content, None)
            .map(|tree| (tree, content.to_string()))
    }

    fn should_index(&self, node: &Node) -> bool {
        self.get_type(node).is_some()
    }

    fn get_range(&self, node: &Node) -> Option<Range> {
        let range = node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_ident_range(&self, node: &Node) -> Option<Range> {
        let ident_node = match node.kind() {
            "class_declaration" | "method_declaration" => node.child_by_field_name("name")?,
            "field_declaration" => {
                let declarator = node
                    .children(&mut node.walk())
                    .find(|n| n.kind() == "variable_declarator")?;
                declarator.child_by_field_name("name")?
            }
            _ => node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier")?,
        };

        let range = ident_node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_package_name(&self, tree: &Tree, source: &str) -> Option<String> {
        let query_str = "(package_declaration (scoped_identifier) @package)";
        ts_helper::get_one(self.get_ts_language(), &tree.root_node(), source, query_str)
    }

    fn get_type(&self, node: &Node) -> Option<NodeType> {
        match node.kind() {
            "class_declaration" => Some(NodeType::Class),
            "interface_declaration" => Some(NodeType::Interface),
            "enum_declaration" => Some(NodeType::Enum),
            "function_declaration" => Some(NodeType::Function),
            "field_declaration" => node.parent().and_then(|parent| match parent.kind() {
                "class_body" => Some(NodeType::Field),
                _ => None,
            }),
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(NodeType::Field) => {
                let query_str =
                    "(field_declaration (variable_declarator name: (identifier) @name))";
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!("({node_kind} name: (identifier) @name)");
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            None => None,
        }
    }

    fn get_extends(&self, node: &Node, source: &str) -> Option<String> {
        let query_str = r#"(superclass (type_identifier) @superclass)"#;
        ts_helper::get_one(self.get_ts_language(), node, source, query_str)
    }

    fn get_implements(&self, node: &Node, source: &str) -> Vec<String> {
        let query_str = r#"(super_interfaces (type_list (type_identifier) @interface))"#;
        ts_helper::get_many(self.get_ts_language(), node, source, query_str)
    }

    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!(
                    r#"
                ({node_kind}
                (modifiers
                    [
                        "public"
                        "private"
                        "protected"
                        "static"
                        "final"
                        "abstract"
                        "synchronized"
                        "native"
                        "strictfp"
                        "transient"
                        "volatile"
                    ] @modifier
                ))
                "#
                );
                ts_helper::get_many(self.get_ts_language(), node, source, &query_str)
            }
            None => Vec::new(),
        }
    }

    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!(
                    r#"
                ({node_kind}
                (modifiers
                    [
                        (marker_annotation name: (identifier) @annotation)
                        (annotation name: (identifier) @annotation)
                    ]
                ))
                "#
                );
                ts_helper::get_many(self.get_ts_language(), node, source, &query_str)
            }
            None => Vec::new(),
        }
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        let query_str = "(groovydoc_comment) @doc";
        ts_helper::get_one(self.get_ts_language(), node, source, query_str)
    }

    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>> {
        if let Some(NodeType::Function) = self.get_type(node) {
            let query_str = "(function_declaration (parameters (parameter) @arg))";
            let params = ts_helper::get_many(self.get_ts_language(), node, source, query_str)
                .into_iter()
                .map(|p| ts_helper::parse_parameter(&p))
                .collect();
            Some(params)
        } else {
            None
        }
    }

    fn get_return(&self, node: &Node, source: &str) -> Option<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(NodeType::Field) => {
                let query_str = "(field_declaration type: (_) @ret)";
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            Some(NodeType::Function) => {
                let query_str = "(function_declaration (type_identifier) @ret)";
                ts_helper::get_one(self.get_ts_language(), node, source, query_str)
            }
            _ => None,
        }
    }
}

mod tests {
    use super::*;

    fn find_node_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_node_by_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn test_get_ident_range() {
        let support = GroovySupport::new();
        let content = "class MyClass {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();

        let support = GroovySupport::new();
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
        let support = GroovySupport::new();
        let content = "package com.example.app\n\nclass Foo {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");

        let node_name = support.get_package_name(&parsed.0, &parsed.1);
        assert_eq!(node_name, Some("com.example.app".to_string()));
    }

    #[test]
    fn test_get_type() {
        let support = GroovySupport::new();

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

        let content = "package com.example.app\n\nenum Color { RED, GREEN, BLUE }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "enum_declaration").unwrap();
        let node_type = support.get_type(&node);
        assert_eq!(node_type, Some(NodeType::Enum));

        let content = "def myFunction() { return 42 }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
        let node_type = support.get_type(&node);
        assert_eq!(node_type, Some(NodeType::Function));

        let content = "package com.example.app\n\nclass Foo { String bar }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "field_declaration").unwrap();
        let node_type = support.get_type(&node);
        assert_eq!(node_type, Some(NodeType::Field));
    }

    #[test]
    fn test_get_short_name() {
        let support = GroovySupport::new();

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

        let content = "package com.example.app\n\nenum Color { RED, GREEN, BLUE }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "enum_declaration").unwrap();
        let short_name = support.get_short_name(&node, &parsed.1);
        assert_eq!(short_name, Some("Color".to_string()));

        let content = "def myFunction() { return 42 }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
        let short_name = support.get_short_name(&node, &parsed.1);
        assert_eq!(short_name, Some("myFunction".to_string()));

        let content = "package com.example.app\n\nclass Foo { String bar }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "field_declaration").unwrap();
        let node_name = support.get_short_name(&node, &parsed.1);
        assert_eq!(node_name, Some("bar".to_string()));
    }

    #[test]
    fn test_get_extends() {
        let support = GroovySupport::new();
        let content = "package com.example.app\n\nclass Foo extends Bar {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");

        let node = find_node_by_kind(parsed.0.root_node(), "superclass").unwrap();
        let node_name = support.get_extends(&node, &parsed.1);
        assert_eq!(node_name, Some("Bar".to_string()));
    }

    #[test]
    fn test_get_implements() {
        let support = GroovySupport::new();
        let content = "package com.example.app\n\nclass Foo implements Bar, Baz {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");

        let node = find_node_by_kind(parsed.0.root_node(), "super_interfaces").unwrap();
        let node_names = support.get_implements(&node, &parsed.1);
        assert_eq!(node_names, vec!["Bar", "Baz"]);
    }

    #[test]
    fn test_get_modifiers() {
        let support = GroovySupport::new();

        let content = "public abstract class Foo {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let modifiers = support.get_modifiers(&node, &parsed.1);
        assert_eq!(modifiers, vec!["public", "abstract"]);

        let content = "class Bar { private static final void test() {} }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let method_node = find_node_by_kind(class_node, "function_declaration").unwrap();
        let modifiers = support.get_modifiers(&method_node, &parsed.1);
        assert_eq!(modifiers, vec!["private", "static", "final"]);

        let content = "class Baz { public static String name }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let field_node = find_node_by_kind(class_node, "field_declaration").unwrap();
        let modifiers = support.get_modifiers(&field_node, &parsed.1);
        assert_eq!(modifiers, vec!["public", "static"]);

        let content = "class Qux {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let modifiers = support.get_modifiers(&node, &parsed.1);
        assert!(modifiers.is_empty());
    }

    #[test]
    fn test_get_annotations() {
        let support = GroovySupport::new();

        let content = "@Component\n@Service\nclass Foo {}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let annotations = support.get_annotations(&node, &parsed.1);
        assert_eq!(annotations, vec!["Component", "Service"]);

        let content = "class Bar {\n    @Override\n    @Deprecated\n    void test() {}\n}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let method_node = find_node_by_kind(class_node, "function_declaration").unwrap();
        let annotations = support.get_annotations(&method_node, &parsed.1);
        assert_eq!(annotations, vec!["Override", "Deprecated"]);

        let content = "class Baz {\n    @Autowired\n    String name\n}";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let class_node = find_node_by_kind(parsed.0.root_node(), "class_declaration").unwrap();
        let field_node = find_node_by_kind(class_node, "field_declaration").unwrap();
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
        let support = GroovySupport::new();

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
            void test() {}
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
        let support = GroovySupport::new();

        let content = "void myFunction(String arg1 = 'test', int arg2) { };";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
        let arguments = support.get_parameters(&node, &parsed.1).unwrap();
        assert_eq!(
            arguments,
            vec![
                (
                    "arg1".to_string(),
                    Some("String".to_string()),
                    Some("test".to_string())
                ),
                ("arg2".to_string(), Some("int".to_string()), None),
            ]
        );
    }

    #[test]
    fn test_get_return() {
        let support = GroovySupport::new();

        let content = "Foo myFunction(String arg1 = 'test', int arg2) { };";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
        let ret = support.get_return(&node, &parsed.1);
        assert_eq!(ret, Some("Foo".to_string()));

        let content = "void myFunction(String arg1 = 'test', int arg2) { };";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "function_declaration").unwrap();
        let ret = support.get_return(&node, &parsed.1);
        assert_eq!(ret, None);

        let content = "class UserService { private String myVar }";
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let node = find_node_by_kind(parsed.0.root_node(), "field_declaration").unwrap();
        let ret = support.get_return(&node, &parsed.1);
        assert_eq!(ret, Some("String".to_string()));
    }
}
