#![allow(unused_imports)]
use super::*;
use crate::KotlinSupport;
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};
use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

#[test]
fn test_extract_call_arguments_all_expression_types() {
    let support = KotlinSupport::new();
    let content = r#"
        class TestClass {
            fun testMethod() {
                val x = 5
                val y = 10
                var flag = true
                val str = "hello"
                val obj: Any = Any()
                val arr = arrayOf(1, 2, 3)
                val nested = SomeClass()
                
                myMethod(
                    x + 3,
                    a + b * 2,
                    obj is String,
                    if (flag) "yes" else "no",
                    i++,
                    myVar,
                    -x,
                    !flag,
                    obj as String,
                    nested.method().field,
                    arr[0],
                    "literal",
                    123,
                    ArrayList<Int>()
                )
            }
        }
    "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "myMethod(");
    let args = support.extract_call_arguments(&parsed.0, &parsed.1, &pos);
    assert!(args.is_some());
    let args = args.unwrap();
    assert_eq!(args.len(), 14);
    assert_eq!(args[0].0, "x + 3");
    assert_eq!(args[1].0, "a + b * 2");
    assert_eq!(args[2].0, "obj is String");
    assert_eq!(args[3].0, "if (flag) \"yes\" else \"no\"");
    assert_eq!(args[4].0, "i++");
    assert_eq!(args[5].0, "myVar");
    assert_eq!(args[6].0, "-x");
    assert_eq!(args[7].0, "!flag");
    assert_eq!(args[8].0, "obj as String");
    assert_eq!(args[9].0, "nested.method().field");
    assert_eq!(args[10].0, "arr[0]");
    assert_eq!(args[11].0, "\"literal\"");
    assert_eq!(args[12].0, "123");
    assert_eq!(args[13].0, "ArrayList<Int>()");
}
