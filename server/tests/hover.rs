use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Position,
        TextDocumentIdentifier, TextDocumentPositionParams, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test]
async fn hover_project_symbol() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("core/src/main/groovy/com/example/core/DataProcessor.groovy"),
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
async fn hover_external_symbol() {
    let server = get_test_server("polyglot-spring").await;

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(25, 24),
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

#[tokio::test]
async fn hover_class() {
    let server = get_test_server("polyglot-spring").await;

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(11, 5),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```java\npackage com.example\n\n@Service\npublic class JavaService\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}

#[tokio::test]
async fn hover_interface() {
    let server = get_test_server("polyglot-spring").await;

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/kotlin/com/example/demo/UserRepository.kt"),
            },
            position: Position::new(5, 24),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```java\npackage com.example\n\npublic interface BaseRepository\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}

#[tokio::test]
async fn hover_method() {
    let server = get_test_server("polyglot-spring").await;

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(31, 45),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```groovy\npackage com.example\n\nString process(String input)\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}
