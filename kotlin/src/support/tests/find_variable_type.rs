#![allow(unused_imports)]
use super::*;
use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};
use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

#[test]
fn test_find_variable_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
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
fn test_find_variable_type_explicit() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar: Bar = Bar()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val items: List<String> = ArrayList()
                items.add("test")
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test(user: User) {
                user.getName()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user.getName");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "user", &pos);
    assert_eq!(var_type, Some("User".to_string()));
}

#[test]
fn test_find_property_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            private val name: String = "test"
            fun test() {
                name.lowercase()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name.lowercase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "name", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_find_property_type_inferred_string() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            private val name = "test"
            fun test() {
                name.lowercase()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name.lowercase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "name", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_infer_integer_literal() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val n = 42
                n.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "n.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "n", &pos);
    assert_eq!(var_type, Some("Int".to_string()));
}

#[test]
fn test_infer_long_literal() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val l = 42L
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
fn test_infer_double_literal() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val d = 3.14
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
fn test_infer_float_literal() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val f = 3.14f
                f.toString()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "f.toString");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "f", &pos);
    assert_eq!(var_type, Some("Float".to_string()));
}

#[test]
fn test_infer_boolean_literal() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val b = true
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
fn test_find_this_type_nested_class() {
    let support = KotlinSupport::new();
    let content = r#"
        class Outer {
            val outerField: String = ""
            inner class Inner {
                val innerField: Int = 0
                fun test() {
                    this.innerField.toString()
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
fn test_find_lambda_parameter_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val items = listOf("a", "b")
                items.forEach { item: String ->
                    item.uppercase()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item.uppercase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "item", &pos);
    assert_eq!(var_type, Some("String".to_string()));
}

#[test]
fn test_val_infer_static_method_call() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val result = Bar.create()
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
fn test_val_infer_chained_method_call() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val result = foo.bar().baz()
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
fn test_find_variable_type_nullable() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar: Bar? = null
                bar?.doSomething()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar?.");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "bar", &pos);
    assert_eq!(var_type, Some("Bar?".to_string()));
}

#[test]
fn test_find_untyped_lambda_parameter_marker() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                items.forEach { item ->
                    item.uppercase()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item.uppercase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "item", &pos);
    assert_eq!(var_type, Some("__cp__:items:forEach:0:0".to_string()));
}

#[test]
fn test_implicit_it_type_marker() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                items.forEach {
                    it.uppercase()
                }
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "it.uppercase");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "it", &pos);
    assert_eq!(var_type, Some("__cp__:items:forEach:0:0".to_string()));
}

#[test]
fn test_val_infer_chain_with_lambda_body_encoding() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val result = items.map { item -> item.toUpperCase() }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#map__lb__item|item#toUpperCase".to_string())
    );
}

#[test]
fn test_val_infer_implicit_it_lambda_body_encoding() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val result = items.map { it.length() }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#map__lb__it|it#length".to_string())
    );
}

#[test]
fn test_val_infer_implicit_it_property_access_body_encoding() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val result = items.map { it.name }
                result.size()
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "result.size");
    let var_type = support.find_variable_type(&parsed.0, &parsed.1, "result", &pos);
    assert_eq!(
        var_type,
        Some("items#map__lb__it|it#name".to_string())
    );
}
