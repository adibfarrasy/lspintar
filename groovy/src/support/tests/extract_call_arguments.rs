#![allow(unused_imports)]

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

#[test]
fn test_extract_call_arguments_all_expression_types() {
    let support = GroovySupport::new();
    let content = r#"
        class TestClass {
            void testMethod() {
                int x = 5, y = 10
                boolean flag = true
                String str = "hello"
                Object obj = new Object()
                def arr = [1, 2, 3]
                def nested = new SomeClass()
                
                myMethod(
                    x = 3,
                    a + b * 2,
                    1..10,
                    obj instanceof String,
                    flag ? "yes" : "no",
                    i++,
                    myVar,
                    -x,
                    !flag,
                    (String) obj,
                    nested.method().field,
                    arr[0],
                    "literal",
                    123,
                    new ArrayList<>()
                )
            }
        }
    "#;

    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "myMethod(");
    let args = support.extract_call_arguments(&parsed.0, &parsed.1, &pos);

    assert!(args.is_some());
    let args = args.unwrap();

    assert_eq!(args.len(), 15);
    assert_eq!(args[0].0, "x = 3");
    assert_eq!(args[1].0, "a + b * 2");
    assert_eq!(args[2].0, "1..10");
    assert_eq!(args[3].0, "obj instanceof String");
    assert_eq!(args[4].0, "flag ? \"yes\" : \"no\"");
    assert_eq!(args[5].0, "i++");
    assert_eq!(args[6].0, "myVar");
    assert_eq!(args[7].0, "-x");
    assert_eq!(args[8].0, "!flag");
    assert_eq!(args[9].0, "(String) obj");
    assert_eq!(args[10].0, "nested.method().field");
    assert_eq!(args[11].0, "arr[0]");
    assert_eq!(args[12].0, "\"literal\"");
    assert_eq!(args[13].0, "123");
    assert_eq!(args[14].0, "new ArrayList<>()");
}
