#![allow(unused_imports)]

use tower_lsp::lsp_types::Position;

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use super::*;

#[test]
fn test_get_method_receiver_type() {
    let support = GroovySupport::new();
    let content = r#"
        interface Foo {
            void doSomething()
        }
        "#;
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let pos = find_position(content, "doSomething");
    let receiver_type =
        support.get_method_receiver_and_params(parsed.0.root_node(), &parsed.1, &pos);
    assert_eq!(receiver_type, Some(("Foo".to_string(), vec![])));
}
