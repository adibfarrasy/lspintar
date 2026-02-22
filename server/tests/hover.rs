use std::env;

use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Position,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test(flavor = "multi_thread")]
async fn hover_project_symbol() {
    let server = get_test_server("groovy-gradle-multi").await;

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

#[tokio::test(flavor = "multi_thread")]
async fn hover_external_symbol() {
    let server = get_test_server("polyglot-spring").await;

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
