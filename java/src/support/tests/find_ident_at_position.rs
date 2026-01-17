#![allow(unused_imports)]

use crate::JavaSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_simple_identifier() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar();
                bar;
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar;");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("bar".to_string(), None)));
}

#[test]
fn test_method_invocation_with_qualifier() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar();
                bar.baz();
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "baz");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("baz".to_string(), Some("bar".to_string()))));
}

#[test]
fn test_field_access_with_qualifier() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar();
                bar.name;
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("name".to_string(), Some("bar".to_string()))));
}

#[test]
fn test_identifier_in_argument_list() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar bar = new Bar();
                System.out.println(bar);
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar)");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("bar".to_string(), None)));
}

#[test]
fn test_identifier_in_variable_declarator() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar myBar = someOtherVar;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "someOtherVar");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("someOtherVar".to_string(), None)));
}

#[test]
fn test_this_qualified_method_invocation() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                this.doSomething();
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("doSomething".to_string(), Some("this".to_string())))
    );
}

#[test]
fn test_chained_method_invocation() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                user.getProfile().getName();
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "getName");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("getName".to_string(), Some("user#getProfile".to_string())))
    );
}

#[test]
fn test_static_method_invocation() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                UserService.createUser();
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "createUser");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("createUser".to_string(), Some("UserService".to_string())))
    );
}

#[test]
fn test_chained_field_access() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                user.profile.name;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "name");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("name".to_string(), Some("user#profile".to_string())))
    );
}

#[test]
fn test_method_parameter_identifier() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test(User user) {
                user;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user;");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("user".to_string(), None)));
}

#[test]
fn test_lambda_parameter_identifier() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<Integer> list = Arrays.asList(1, 2, 3);
                list.forEach(item -> {
                    System.out.println(item);
                });
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item)");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("item".to_string(), None)));
}

#[test]
fn test_constructor_type_in_new_expression() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                ArrayList<String> list = new ArrayList<String>();
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "ArrayList<String>()");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("ArrayList".to_string(), None)));
}

#[test]
fn test_type_argument_in_generics() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<MyClass> list;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_nested_type_arguments_in_generics() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Map<String, UserProfile> map;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "UserProfile");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("UserProfile".to_string(), None)));
}

#[test]
fn test_cast_expression_type() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                Object obj = new Object();
                MyClass result = (MyClass) obj;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass)");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_import_statement_type() {
    let support = JavaSupport::new();
    let content = r#"
        import com.example.Bar;

        class Foo {
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "Bar");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("Bar".to_string(), Some("com.example".to_string())))
    );
}

#[test]
fn test_nested_qualifier_in_chain() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                user.getProfile().name;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    // deepest qualifier
    let pos = find_position(content, "user");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("user".to_string(), None)));

    // middle of chain
    let pos = find_position(content, "getProfile");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("getProfile".to_string(), Some("user".to_string())))
    );

    // field access at end of chain
    let pos = find_position(content, "name");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("name".to_string(), Some("user#getProfile".to_string())))
    );
}

#[test]
fn test_simple_constructor_in_chain() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                new MyClass().process(new HashMap<>()).message;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    // constructor type
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));

    // method on constructor
    let pos = find_position(content, "process");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("process".to_string(), Some("MyClass".to_string())))
    );

    // field at end of chain
    let pos = find_position(content, "message");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("message".to_string(), Some("MyClass#process".to_string())))
    );
}

#[test]
fn test_scoped_constructor_in_chain() {
    let support = JavaSupport::new();
    let content = r#"
        class Foo {
            void test() {
                new Outer.Inner().process(new HashMap<>()).message;
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    // outer qualifier
    let pos = find_position(content, "Outer");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Outer".to_string(), None)));

    // inner constructor type
    let pos = find_position(content, "Inner");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("Inner".to_string(), Some("Outer".to_string())))
    );

    // method on constructor
    let pos = find_position(content, "process");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("process".to_string(), Some("Outer#Inner".to_string())))
    );

    // field at end of chain
    let pos = find_position(content, "message");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some((
            "message".to_string(),
            Some("Outer#Inner#process".to_string())
        ))
    );
}
