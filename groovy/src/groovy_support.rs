use lsp_core::{
    language_support::{IdentResult, LanguageSupport, ParameterResult, ParseResult},
    languages::Language,
    node_types::NodeType,
    ts_helper::{self, node_contains_position},
};
use std::{fs, path::Path, sync::Mutex};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::constants::GROOVY_IMPLICIT_IMPORTS;

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

    fn try_extract_ident_result(
        &self,
        query: &Query,
        match_: &QueryMatch,
        content: &str,
        position: &Position,
        name: &str,
        qual: Option<&str>,
    ) -> Option<IdentResult> {
        let name_idx = query.capture_index_for_name(name);
        let name_cap = match_.captures.iter().find(|c| Some(c.index) == name_idx)?;

        if !node_contains_position(&name_cap.node, position) {
            return None;
        }

        let ident = name_cap
            .node
            .utf8_text(content.as_bytes())
            .ok()?
            .to_string();

        let qualifier = if let Some(qual_name) = qual {
            let qual_idx = query.capture_index_for_name(qual_name);
            match_
                .captures
                .iter()
                .find(|c| Some(c.index) == qual_idx)
                .and_then(|cap| cap.node.utf8_text(content.as_bytes()).ok())
                .map(|s| s.to_string())
        } else {
            None
        };

        Some((ident, qualifier))
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

    fn get_package_name(&self, tree: &Tree, content: &str) -> Option<String> {
        let query_str = "(package_declaration (scoped_identifier) @package)";
        ts_helper::get_one(
            self.get_ts_language(),
            &tree.root_node(),
            content,
            query_str,
        )
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

    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String> {
        let query_str = "(import_declaration) @doc";
        let explicit_imports =
            ts_helper::get_many(self.get_ts_language(), &tree.root_node(), source, query_str)
                .into_iter()
                .map(|i| i.strip_prefix("import ").unwrap_or_default().to_string())
                .collect::<Vec<String>>();

        GROOVY_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(explicit_imports)
            .collect()
    }

    fn get_type_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        let query_text = "(type_identifier) @identifier";
        let query = Query::new(&tree.language(), query_text).ok()?;

        let mut result = None;

        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, tree.root_node(), content.as_bytes())
            .find(|match_| {
                for capture in match_.captures.iter() {
                    let node = capture.node;
                    if node_contains_position(&node, position) {
                        let ident_name = node
                            .utf8_text(content.as_bytes())
                            .unwrap_or_default()
                            .to_string();

                        result = Some(ident_name);
                    }
                }

                result.is_some()
            });

        result
    }

    fn get_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let query_text = r#"
            (expression_statement (identifier) @trivial_case)

            (method_invocation
                object: (_) @method_qualifier
                name: (identifier) @method_name)

            (method_invocation
                object: (this) @this_qualifier
                name: (identifier) @this_method_name)

            (field_access
                object: (_) @field_qualifier
                field: (identifier) @field_name)

            (argument_list (identifier) @arg_name)

            (variable_declarator (identifier) @var_decl)
        "#;
        let query = Query::new(&tree.language(), query_text).ok()?;
        let mut cursor = QueryCursor::new();
        let mut result = None;

        cursor
            .matches(&query, tree.root_node(), content.as_bytes())
            .for_each(|m| {
                if result.is_some() {
                    return;
                }

                println!("Match pattern_index: {}", m.pattern_index);
                for cap in m.captures {
                    let name = query.capture_names()[cap.index as usize];
                    let text = cap.node.utf8_text(content.as_bytes()).ok();
                    println!("  Capture '{}': {:?}", name, text);
                }

                vec![
                    ("trivial_case", None),
                    ("method_name", Some("method_qualifier")),
                    ("this_method_name", Some("this_qualifier")),
                    ("field_name", Some("field_qualifier")),
                    ("arg_name", None),
                    ("var_decl", None),
                ]
                .into_iter()
                .for_each(|(name, qual)| {
                    if let Some(r) =
                        self.try_extract_ident_result(&query, &m, content, position, name, qual)
                    {
                        result = Some(r);
                        return;
                    }
                });
            });

        result
    }
}

mod tests {
    use tree_sitter::Point;

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

    #[test]
    fn test_get_imports() {
        let support = GroovySupport::new();
        let content = "package com.example.app\n\nimport com.example.Foo\nimport java.lang.*";
        let parsed = support.parse_str(&content).expect("cannot parse content");

        let node_names = support.get_imports(&parsed.0, &parsed.1);
        assert_eq!(
            node_names,
            GROOVY_IMPLICIT_IMPORTS
                .iter()
                .map(|s| s.to_string())
                .chain(vec![
                    "com.example.Foo".to_string(),
                    "java.lang.*".to_string()
                ])
                .collect::<Vec<String>>()
        );
    }

    fn find_position(content: &str, marker: &str) -> Position {
        content
            .lines()
            .enumerate()
            .find_map(|(line_num, line)| {
                line.find(marker)
                    .map(|col| Position::new(line_num as u32, col as u32))
            })
            .expect(&format!("Marker '{}' not found", marker))
    }

    #[test]
    fn test_get_ident_at_position() {
        let support = GroovySupport::new();

        let content = r#"
        class Foo {
            void test() {
                def bar = new Bar()
                bar;
            }
        }
        "#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "bar;");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("bar".to_string(), None)));

        let content = r#"
        class Foo {
            void test() {
                def bar = new Bar()
                bar.baz() 
            }
        }
        "#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "baz");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("baz".to_string(), Some("bar".to_string()))));

        let content = r#"
        class Foo {
            void test() {
                def bar = new Bar()
                bar.name 
            }
        }
        "#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "name");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("name".to_string(), Some("bar".to_string()))));

        let content = r#"
        class Foo {
            void test() {
                def bar = new Bar()
                println(bar)
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "bar)"); // Second 'bar'
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("bar".to_string(), None)));

        let content = r#"
        class Foo {
            void test() {
                Bar myBar = someOtherVar
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "someOtherVar");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("someOtherVar".to_string(), None)));

        let content = r#"
        class Foo {
            void test() {
                this.doSomething()
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "doSomething");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(
            ident,
            Some(("doSomething".to_string(), Some("this".to_string())))
        );

        let content = r#"
        class Foo {
            void test() {
                user.getProfile().getName()
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "getName");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(
            ident,
            Some(("getName".to_string(), Some("user.getProfile()".to_string())))
        );

        let content = r#"
        class Foo {
            void test() {
                UserService.createUser()
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "createUser");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(
            ident,
            Some(("createUser".to_string(), Some("UserService".to_string())))
        );

        let content = r#"
        class Foo {
            void test() {
                user.profile.name
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "name");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(
            ident,
            Some(("name".to_string(), Some("user.profile".to_string())))
        );

        let content = r#"
        class Foo {
            void test(User user) {
                user;
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "user;");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("user".to_string(), None)));

        let content = r#"
        class Foo {
            void test() {
                def list = [1, 2, 3]
                list.each { item -> 
                    println(item)
                }
            }
        }"#;
        let parsed = support.parse_str(&content).expect("cannot parse content");
        let pos = find_position(content, "item)");
        let ident = support.get_ident_at_position(&parsed.0, &parsed.1, &pos);
        assert_eq!(ident, Some(("item".to_string(), None)));
    }
}
