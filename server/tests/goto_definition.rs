use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        GotoDefinitionParams, GotoDefinitionResponse, Location, PartialResultParams, Position,
        Range, TextDocumentIdentifier, TextDocumentPositionParams, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

#[tokio::test]
async fn gtd_simple() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("app/src/main/groovy/com/example/app/Application.groovy"),
            },
            position: Position::new(6, 35),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 6, character: 6 },
            end: Position { line: 6, character: 20 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_static_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(47, 37),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("core/src/main/groovy/com/example/core/DataProcessor.groovy"),
        Range {
            start: Position { line: 5, character: 21 },
            end: Position { line: 5, character: 35 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_this_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(51, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 19, character: 9 },
            end: Position { line: 19, character: 16 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_this_super_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(54, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("core/src/main/groovy/com/example/core/BaseService.groovy"),
        Range {
            start: Position { line: 7, character: 11 },
            end: Position { line: 7, character: 22 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_instance_member_access() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(63, 49),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
        Range {
            start: Position { line: 24, character: 22 },
            end: Position { line: 24, character: 29 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_resolve_chain() {
    let server = get_test_server("groovy-gradle-multi").await;
    let controller_uri =
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: controller_uri.clone() },
            position: Position::new(69, 44),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let expected = Location::new(
        server.uri("core/src/main/groovy/com/example/core/DataProcessResult.groovy"),
        Range {
            start: Position { line: 8, character: 11 },
            end: Position { line: 8, character: 18 },
        },
    );
    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(expected.clone()));

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: controller_uri },
            position: Position::new(71, 54),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(expected));
}

#[tokio::test]
async fn gtd_method_overloading() {
    let server = get_test_server("groovy-gradle-multi").await;
    let controller_uri =
        server.uri("api/src/main/groovy/com/example/api/UserController.groovy");

    let cases = [
        (74u32, 28u32),
        (76, 32),
        (79, 36),
        (81, 40),
    ];

    for (cursor_line, expected_line) in cases {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: controller_uri.clone() },
                position: Position::new(cursor_line, 14),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let result = server.backend.goto_definition(params).await.unwrap();
        assert!(result.is_some(), "cursor line {cursor_line} returned None");

        let location = Location::new(
            controller_uri.clone(),
            Range {
                start: Position { line: expected_line, character: 17 },
                end: Position { line: expected_line, character: 32 },
            },
        );

        assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
    }
}

#[tokio::test]
async fn gtd_goto_superclass() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(6, 30),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("core/src/main/groovy/com/example/core/BaseService.groovy"),
        Range {
            start: Position { line: 4, character: 15 },
            end: Position { line: 4, character: 26 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_goto_interface() {
    let server = get_test_server("groovy-gradle-multi").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("api/src/main/groovy/com/example/api/UserController.groovy"),
            },
            position: Position::new(6, 53),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("core/src/main/groovy/com/example/core/DataProcessor.groovy"),
        Range {
            start: Position { line: 4, character: 10 },
            end: Position { line: 4, character: 23 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_goto_property() {
    let server = get_test_server("polyglot-spring").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(28, 29),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
        Range {
            start: Position { line: 11, character: 16 },
            end: Position { line: 11, character: 16 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_goto_data_class_field() {
    let server = get_test_server("polyglot-spring").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(43, 32),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        server.uri("src/main/kotlin/com/example/demo/User.kt"),
        Range {
            start: Position { line: 4, character: 8 },
            end: Position { line: 4, character: 12 },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test]
async fn gtd_resolve_chain_external() {
    let server = get_test_server("polyglot-spring").await;

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("src/main/groovy/com/example/demo/Controller.groovy"),
            },
            position: Position::new(25, 36),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = match result.unwrap() {
        GotoDefinitionResponse::Scalar(loc) => loc,
        _ => panic!("Expected scalar location"),
    };

    assert!(
        location
            .uri
            .path()
            .ends_with("org/apache/commons/lang3/StringUtils.java")
    );

    assert_eq!(location.range.start.line, 536);
    assert_eq!(location.range.start.character, 25);
}
