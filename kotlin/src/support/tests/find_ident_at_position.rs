#![allow(unused_imports)]

use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_types::NodeType};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_simple_identifier() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
                bar // marker
            }
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar /");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("bar".to_string(), None)));
}

#[test]
fn test_method_invocation_with_qualifier() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
                bar.baz()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
                bar.name
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val bar = Bar()
                println(bar)
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "bar)");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("bar".to_string(), None)));
}

#[test]
fn test_identifier_in_variable_declarator() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val myBar = someOtherVar
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "someOtherVar");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("someOtherVar".to_string(), None)));
}

#[test]
fn test_this_qualified_method_invocation() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                this.doSomething()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                user.getProfile().getName()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                UserService.createUser()
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                user.profile.name
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test(user: User) {
                user // marker
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "user /");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("user".to_string(), None)));
}

#[test]
fn test_lambda_parameter_identifier() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val list = listOf(1, 2, 3)
                list.forEach { item ->
                    println(item)
                }
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "item)");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("item".to_string(), None)));
}

#[test]
fn test_constructor_type_in_new_expression() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val list = ArrayList<String>()
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "ArrayList");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("ArrayList".to_string(), None)));
}

#[test]
fn test_type_argument_in_generics() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val list: List<MyClass>
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_nested_type_arguments_in_generics() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val map: Map<String, UserProfile>
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "UserProfile");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("UserProfile".to_string(), None)));
}

#[test]
fn test_cast_expression_type() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                val obj: Any = Any()
                val result = obj as MyClass
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_import_statement_type() {
    let support = KotlinSupport::new();
    let content = r#"
        import com.example.Bar

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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                user.getProfile().name
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                MyClass().process(HashMap<String, String>()).message
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
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test() {
                Outer.Inner().process(HashMap<String, String>()).message
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

#[test]
fn test_class_declaration_name() {
    let support = KotlinSupport::new();
    let content = r#"
        class MyClass {
            fun test() {}
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_interface_declaration_name() {
    let support = KotlinSupport::new();
    let content = r#"
        interface MyInterface {
            fun test()
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyInterface");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyInterface".to_string(), None)));
}

#[test]
fn test_function_declaration_name() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun myFunction() {
                println("test")
            }
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "myFunction");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("myFunction".to_string(), None)));
}

#[test]
fn test_property_declaration_name() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            val myProperty: String = "test"
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "myProperty");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("myProperty".to_string(), None)));
}

#[test]
fn test_delegation_superclass() {
    let support = KotlinSupport::new();
    let content = r#"
        class MyClass : BaseClass() {
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "BaseClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("BaseClass".to_string(), None)));
}

#[test]
fn test_delegation_interface() {
    let support = KotlinSupport::new();
    let content = r#"
        class MyClass : MyInterface {
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyInterface");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyInterface".to_string(), None)));
}

#[test]
fn test_delegation_multiple() {
    let support = KotlinSupport::new();
    let content = r#"
        class MyClass : BaseClass(), Interface1, Interface2 {
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    let pos = find_position(content, "BaseClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("BaseClass".to_string(), None)));

    let pos = find_position(content, "Interface1");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Interface1".to_string(), None)));

    let pos = find_position(content, "Interface2");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Interface2".to_string(), None)));
}

#[test]
fn test_return_type_detection() {
    let support = KotlinSupport::new();
    let content = r#"
        class Foo {
            fun test(): String {
                return "hello"
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("String".to_string(), None)));
}
