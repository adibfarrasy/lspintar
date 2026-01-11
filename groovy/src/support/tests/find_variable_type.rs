#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

#[allow(dead_code)]
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
fn test_find_variable_type() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar()
                bar.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar.");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "bar", &pos);
    assert_eq!(var_type, Some("Bar".to_string()));
}

#[test]
fn test_find_variable_type_with_generics() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<String> items = []
                items.add("test");
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "items.add");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "items", &pos);
    assert_eq!(var_type, Some("List<String>".to_string()));
}

#[test]
fn test_find_parameter_type() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test(User user) {
                user.getName();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user.getName");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "user", &pos);
    assert_eq!(var_type, Some("User".to_string()));
}

#[test]
fn test_find_field_type() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            private String name

            void test() {
                name.toLowerCase();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name.toLowerCase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "name", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_find_this_type_nested_class() {
    let support = GroovySupport::new();
    let content = r#"
        class Outer {
            String outerField

            class Inner {
                Integer innerField

                void test() {
                    this.innerField.toString();
                }
            }
        }
        "#;

    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "this.innerField");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "this", &pos);

    assert_eq!(var_type, Some("Inner".to_string()));
}
