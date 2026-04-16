use std::env;

use tower_lsp::lsp_types::{CompletionParams, CompletionResponse, DidChangeTextDocumentParams, TextDocumentContentChangeEvent, VersionedTextDocumentIdentifier};
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        PartialResultParams, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test]
async fn completion_chain_with_import() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(25, 36),
        },
        context: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.completion(params).await.unwrap();
    assert!(result.is_some());

    match result.unwrap() {
        CompletionResponse::Array(items) => {
            assert!(!items.is_empty());

            assert!(
                items
                    .iter()
                    .map(|i| i.label.clone())
                    .any(|l| l == "capitalize".to_string())
            )
        }
        _ => panic!("Invalid completion response"),
    }
}

#[tokio::test]
async fn completion_prefix_with_import() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(25, 31),
        },
        context: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.completion(params).await.unwrap();
    assert!(result.is_some());

    match result.unwrap() {
        CompletionResponse::Array(items) => {
            assert!(!items.is_empty());

            assert!(
                items
                    .iter()
                    .map(|i| i.label.clone())
                    .any(|l| l == "StringUtils".to_string())
            )
        }
        _ => panic!("Invalid completion response"),
    }
}

// Cursor at end of "capitalize" with no dot before it.
// StringUtils#capitalize is a class member and must not appear in naked prefix completion.
#[tokio::test]
async fn completion_prefix_excludes_class_members() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join(
                    "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/CompletionTest.groovy",
                ))
                .expect("cannot parse root URI"),
            },
            position: Position::new(9, 18),
        },
        context: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.completion(params).await.unwrap();
    match result {
        None => {
            // No completions at all – capitalize is absent, which is the expected outcome.
        }
        Some(CompletionResponse::Array(items)) => {
            assert!(
                !items.iter().any(|i| i.label == "capitalize"),
                "class member 'capitalize' must not appear in naked prefix completion"
            );
        }
        _ => panic!("Invalid completion response"),
    }
}

// At prefix "groovy", the local variable groovyResult must appear before the global GroovyService.
#[tokio::test]
async fn completion_locals_before_globals() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join(
                    "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/CompletionTest.groovy",
                ))
                .expect("cannot parse root URI"),
            },
            position: Position::new(15, 14),
        },
        context: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.completion(params).await.unwrap();
    match result.unwrap() {
        CompletionResponse::Array(items) => {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            let local_pos = labels.iter().position(|&l| l == "groovyResult");
            let global_pos = labels.iter().position(|&l| l == "GroovyService");
            assert!(local_pos.is_some(), "groovyResult (local) must be in results");
            if let (Some(local_idx), Some(global_idx)) = (local_pos, global_pos) {
                assert!(
                    local_idx < global_idx,
                    "local 'groovyResult' (index {local_idx}) must appear before global 'GroovyService' (index {global_idx})"
                );
            }
        }
        _ => panic!("Invalid completion response"),
    }
}

// Closure is in groovy.lang.*, which is an implicit import for Groovy files.
// Chain completion on a Closure variable must return results without an explicit import.
#[tokio::test]
async fn completion_chain_via_implicit_import() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let uri = Url::from_file_path(root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/CompletionTest.groovy",
    ))
    .expect("cannot parse root URI");

    // Send did_change so the server uses in-memory content at the exact cursor position.
    let content = std::fs::read_to_string(root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/CompletionTest.groovy",
    ))
    .expect("cannot read fixture");
    server
        .backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 1,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content,
            }],
        })
        .await;

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position::new(21, 18),
        },
        context: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.completion(params).await.unwrap();
    match result.unwrap() {
        CompletionResponse::Array(items) => {
            assert!(
                !items.is_empty(),
                "chain completion on Closure (via implicit groovy.lang.*) must return results"
            );
        }
        _ => panic!("Invalid completion response"),
    }
}
