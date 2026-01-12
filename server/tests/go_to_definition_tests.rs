use std::env;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use server::{Repository, server::Backend};
use tower_lsp::LanguageServer;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Range};
use tower_lsp::{
    LspService,
    lsp_types::{
        GotoDefinitionParams, InitializeParams, InitializedParams, PartialResultParams, Position,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
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

async fn get_server() -> TestServer {
    TestServer::new().await
}

#[tokio::test]
async fn test_simple() {
    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/app/src/main/groovy/com/example/app/Application.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(7, 35),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
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

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn test_static_member() {
    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(35, 37),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 5,
                character: 21,
            },
            end: Position {
                line: 5,
                character: 35,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn test_this_member() {
    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(42, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 19,
                character: 9,
            },
            end: Position {
                line: 19,
                character: 16,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn test_this_super_member() {
    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(45, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 7,
                character: 11,
            },
            end: Position {
                line: 7,
                character: 22,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn test_instance_member_access() {
    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(54, 49),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
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

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn test_resolve_chain() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    let server = get_server().await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(60, 44),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessResult.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 8,
                character: 11,
            },
            end: Position {
                line: 8,
                character: 18,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(62, 54),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "/Users/adibf/Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessResult.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 8,
                character: 11,
            },
            end: Position {
                line: 8,
                character: 18,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}
