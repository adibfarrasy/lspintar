use std::env;
use std::sync::Arc;

use lspintar_server::{Repository, server::Backend};
use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer, LspService,
    lsp_types::{
        Hover, HoverContents, HoverParams, InitializeParams, InitializedParams, MarkupContent,
        MarkupKind, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    },
};
use uuid::Uuid;

struct TestServer {
    backend: Backend,
}

impl TestServer {
    async fn new(fixture: &str) -> Self {
        let db_name = Uuid::new_v4();
        let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
        let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
        let (service, _socket) = LspService::new(|client| Backend::new(client, repo.clone()));
        let backend = Backend::new(service.inner().client.clone(), repo.clone());

        let root = env::current_dir().expect("cannot get current dir");

        let mut init_params = InitializeParams::default();
        init_params.root_uri = Some(
            Url::from_file_path(root.join("tests/fixtures").join(fixture))
                .expect("cannot parse root URI"),
        );
        backend.initialize(init_params).await.unwrap();
        backend.initialized(InitializedParams {}).await;

        Self { backend }
    }
}

#[tokio::test]
async fn test_hover_project_symbol() {
    let server = TestServer::new("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(4, 11),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```groovy\npackage com.example.core\n\ninterface DataProcessor\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}

#[tokio::test]
async fn test_hover_external_symbol() {
    let server = TestServer::new("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(24, 24),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```java\npackage org.apache.commons.lang3\n\npublic class StringUtils\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}
