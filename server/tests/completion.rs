use std::env;

use tower_lsp::lsp_types::{CompletionParams, CompletionResponse};
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        PartialResultParams, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test(flavor = "multi_thread")]
async fn test_completion_chain_with_import() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(24, 36),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_completion_prefix_with_import() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(24, 31),
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
