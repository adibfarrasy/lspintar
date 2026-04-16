#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

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
fn test_def_infer_constructor() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def x = new Bar()
                x.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "x.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "x", &pos);
    assert_eq!(var_type, Some("Bar".to_string()));
}

#[test]
fn test_def_infer_generic_constructor() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def items = new ArrayList<String>()
                items.add("test")
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "items.add");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "items", &pos);
    assert_eq!(var_type, Some("ArrayList<String>".to_string()));
}

#[test]
fn test_def_infer_string_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def s = "hello"
                s.toUpperCase()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "s.toUpperCase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "s", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_def_infer_integer_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def n = 42
                n.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "n.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "n", &pos);
    assert_eq!(var_type, Some("Integer".to_string()));
}

#[test]
fn test_def_infer_long_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def l = 42L
                l.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "l.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "l", &pos);
    assert_eq!(var_type, Some("Long".to_string()));
}

#[test]
fn test_def_infer_bigdecimal_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def d = 3.14
                d.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "d.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "d", &pos);
    assert_eq!(var_type, Some("BigDecimal".to_string()));
}

#[test]
fn test_def_infer_double_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def d = 3.14d
                d.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "d.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "d", &pos);
    assert_eq!(var_type, Some("Double".to_string()));
}

#[test]
fn test_def_infer_boolean_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def b = true
                b.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "b.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "b", &pos);
    assert_eq!(var_type, Some("Boolean".to_string()));
}

#[test]
fn test_def_infer_list_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def items = [1, 2, 3]
                items.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "items.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "items", &pos);
    assert_eq!(var_type, Some("List".to_string()));
}

#[test]
fn test_def_infer_map_literal() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def config = [key: "value"]
                config.get("key")
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "config.get");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "config", &pos);
    assert_eq!(var_type, Some("Map".to_string()));
}

#[test]
fn test_def_infer_static_method_call() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = Bar.create()
                result.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(var_type, Some("Bar#create".to_string()));
}

#[test]
fn test_def_infer_chained_method_call() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = foo.bar().baz()
                result.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(var_type, Some("foo#bar#baz".to_string()));
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

#[test]
fn test_closure_param_trailing_syntax() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                items.each { item ->
                    item.doSomething()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "item", &pos);
    assert_eq!(var_type, Some("__cp__:items:each:0:0".to_string()));
}

#[test]
fn test_closure_param_in_argument_list() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                items.each({ item ->
                    item.doSomething()
                })
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "item", &pos);
    assert_eq!(var_type, Some("__cp__:items:each:0:0".to_string()));
}

#[test]
fn test_closure_param_explicit_type_ignored() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                items.each { String item ->
                    item.doSomething()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "item", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_def_infer_chain_with_closure_body_encoding() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = items.collect { item -> item.toUpperCase() }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#collect__lb__item|item#toUpperCase".to_string())
    );
}

#[test]
fn test_implicit_it_type_marker() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                items.each {
                    it.doSomething()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "it.doSomething");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "it", &pos);
    assert_eq!(var_type, Some("__cp__:items:each:0:0".to_string()));
}

#[test]
fn test_def_infer_chain_with_implicit_it_body_encoding() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = items.collect { it.toUpperCase() }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#collect__lb__it|it#toUpperCase".to_string())
    );
}

#[test]
fn test_def_infer_chain_with_field_access_body_encoding() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = items.collect { it.name }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#collect__lb__it|it#name".to_string())
    );
}

#[test]
fn test_find_variable_type_cursor_after_dot() {
    // When cursor is right after "foo." (no method name typed yet), tree-sitter
    // parses the dot as an ERROR node. The cursor position falls on the parent
    // block node. reference_byte must be derived from the cursor position, not
    // the block's start_byte, so the variable declaration is still found.
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<String> items = []
                items.
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    // Position right after "items." — at the character past the dot.
    let pos = find_position(content, "items.");
    let dot_pos = Position::new(pos.line, pos.character + "items.".len() as u32);
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "items", &dot_pos);
    assert_eq!(var_type, Some("List<String>".to_string()));
}
