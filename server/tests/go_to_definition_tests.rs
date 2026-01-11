use std::env;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use serial_test::serial;
use server::{Repository, server::Backend};
use tokio::sync::OnceCell;
use tower_lsp::LanguageServer;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Range};
use tower_lsp::{
    LspService,
    lsp_types::{
        GotoDefinitionParams, InitializeParams, InitializedParams, PartialResultParams, Position,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

static TEST_SERVER: OnceCell<TestServer> = OnceCell::const_new();

struct TestServer {
    backend: Backend,
}

impl TestServer {
    async fn new() -> Self {
        let db_dir = "file::memory:?cache=shared";
        let repo = Arc::new(Repository::new(db_dir).await.unwrap());
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

async fn get_server() -> &'static TestServer {
    TEST_SERVER
        .get_or_init(|| async { TestServer::new().await })
        .await
}

#[tokio::test]
#[serial(groovy_parser)]
async fn test_goto_definition_simple() {
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
                line: 5,
                character: 6,
            },
            end: Position {
                line: 5,
                character: 20,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
#[serial(groovy_parser)]
async fn test_goto_definition_static_member() {
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
#[serial(groovy_parser)]
async fn test_goto_definition_this_member() {
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
                line: 18,
                character: 9,
            },
            end: Position {
                line: 18,
                character: 16,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}
