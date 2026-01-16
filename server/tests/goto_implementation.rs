use std::env;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use server::{Repository, server::Backend};
use tower_lsp::{
    LanguageServer, LspService,
    lsp_types::{
        InitializeParams, InitializedParams, Location, PartialResultParams, Position, Range,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
        request::GotoImplementationParams, request::GotoImplementationResponse,
    },
};
use uuid::Uuid;

struct TestServer {
    backend: Backend,
}

impl TestServer {
    async fn new() -> Self {
        let db_name = Uuid::new_v4();
        let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
        let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
        let (service, _socket) = LspService::new(|client| Backend::new(client, repo.clone()));
        let backend = Backend::new(service.inner().client.clone(), repo.clone());

        let root = env::current_dir().expect("cannot get current dir");

        let mut init_params = InitializeParams::default();
        init_params.root_uri = Some(
            Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi"))
                .expect("cannot parse root URI"),
        );
        backend.initialize(init_params).await.unwrap();
        backend.initialized(InitializedParams {}).await;

        Self { backend }
    }
}

#[tokio::test]
async fn test_interface_impl() {
    let server = TestServer::new().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(4, 11),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 6,
                character: 6,
            },
            end: Position {
                line: 6,
                character: 20,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}

#[tokio::test]
async fn test_superclass_extends() {
    let server = TestServer::new().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(4, 16),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 6,
                character: 6,
            },
            end: Position {
                line: 6,
                character: 20,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}

#[tokio::test]
async fn test_method_implementation() {
    let server = TestServer::new().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(7, 23),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_implementation(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 24,
                character: 22,
            },
            end: Position {
                line: 24,
                character: 29,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoImplementationResponse::from(location));
}
