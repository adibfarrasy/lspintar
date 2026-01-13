#![allow(unused_imports)]

use crate::GroovySupport;
use lsp_core::language_support::LanguageSupport;

use crate::constants::GROOVY_IMPLICIT_IMPORTS;

use super::*;

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
