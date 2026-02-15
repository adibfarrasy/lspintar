#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

#[test]
fn test_simple_identifier() {
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
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("bar".to_string(), None)));
}

#[test]
fn test_method_invocation_with_qualifier() {
    let support = GroovySupport::new();
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
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("baz".to_string(), Some("bar".to_string()))));
}

#[test]
fn test_field_access_with_qualifier() {
    let support = GroovySupport::new();
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
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("name".to_string(), Some("bar".to_string()))));
}

#[test]
fn test_identifier_in_argument_list() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def bar = new Bar()
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                Bar myBar = someOtherVar
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "someOtherVar");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("someOtherVar".to_string(), None)));
}

#[test]
fn test_this_qualified_method_invocation() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
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
    let support = GroovySupport::new();
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
fn test_closure_parameter_identifier() {
    let support = GroovySupport::new();
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
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("item".to_string(), None)));
}

#[test]
fn test_constructor_type_in_new_expression() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def list = new ArrayList<String>()
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "ArrayList");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("ArrayList".to_string(), None)));
}

#[test]
fn test_type_argument_in_generics() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                List<MyClass> list
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_nested_type_arguments_in_generics() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                Map<String, UserProfile> map
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "UserProfile");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("UserProfile".to_string(), None)));
}

#[test]
fn test_cast_expression_type() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                def result = (MyClass) obj
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "MyClass");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("MyClass".to_string(), None)));
}

#[test]
fn test_import_statement_type() {
    let support = GroovySupport::new();
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
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
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                new Class().process([key: 'value']).message
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");

    // constructor type
    let pos = find_position(content, "Class");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Class".to_string(), None)));

    // method on constructor
    let pos = find_position(content, "process");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("process".to_string(), Some("Class".to_string())))
    );

    // field at end of chain
    let pos = find_position(content, "message");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("message".to_string(), Some("Class#process".to_string())))
    );
}

#[test]
fn test_scoped_constructor_in_chain() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                new Outer.Inner().process([key: 'value']).message
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
fn test_nested_method_calls() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            void test() {
                new MyClass().process(new HashMap()).message
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "message");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(
        ident,
        Some(("message".to_string(), Some("MyClass#process".to_string())))
    );
}

#[test]
fn test_return_type_detection() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            String test() {
                return "hello"
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "String");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("String".to_string(), None)));
}

#[test]
fn test_annotation_on_class() {
    let support = GroovySupport::new();
    let content = r#"
        @Controller
        class Foo {
            String test() {
                return "hello"
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "Controller");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Controller".to_string(), None)));
}

#[test]
fn test_annotation_on_method() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            @GetMapping("/test")
            String test() {
                return "hello"
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "GetMapping");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("GetMapping".to_string(), None)));
}

#[test]
fn test_annotation_on_field() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            @Autowired
            String service
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "Autowired");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("Autowired".to_string(), None)));
}

#[test]
fn test_annotation_with_parameters() {
    let support = GroovySupport::new();
    let content = r#"
        class Foo {
            @RequestMapping(value = "/api", method = RequestMethod.GET)
            String test() {
                return "hello"
            }
        }"#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "RequestMapping");
    let ident = support.find_ident_at_position(&parsed.0, &parsed.1, &pos);
    assert_eq!(ident, Some(("RequestMapping".to_string(), None)));
}
