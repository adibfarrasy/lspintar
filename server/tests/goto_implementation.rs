use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        Location, PartialResultParams, Position, Range, TextDocumentIdentifier,
        TextDocumentPositionParams, WorkDoneProgressParams, request::GotoImplementationParams,
        request::GotoImplementationResponse,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test]
async fn gti_interface_impl() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("core/src/main/groovy/com/example/core/DataProcessor.groovy"),
            },
            position: Position::new(4, 11),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 6, character: 6 },
            end: Position { line: 6, character: 20 },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}

#[tokio::test]
async fn gti_superclass_extends() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("core/src/main/groovy/com/example/core/BaseService.groovy"),
            },
            position: Position::new(4, 16),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 6, character: 6 },
            end: Position { line: 6, character: 20 },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}

#[tokio::test]
async fn gti_method_implementation() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("core/src/main/groovy/com/example/core/DataProcessor.groovy"),
            },
            position: Position::new(7, 23),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 24, character: 22 },
            end: Position { line: 24, character: 29 },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}
