#![allow(unused_imports)]

use crate::{JavaSupport, constants::JAVA_IMPLICIT_IMPORTS};
use lsp_core::{language_support::LanguageSupport, node_kind::NodeKind};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

use super::*;

#[test]
fn test_get_imports() {
    let support = JavaSupport::new();
    let content = "package com.example.app;\n\nimport com.example.Foo;\nimport java.util.*;";
    let parsed = support.parse_str(&content).expect("cannot parse content");
    let node_names = support.get_imports(&parsed.0, &parsed.1);
    assert_eq!(
        node_names,
        JAVA_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(vec![
                "com.example.Foo".to_string(),
                "java.util.*".to_string()
            ])
            .collect::<Vec<String>>()
    );
}
