use std::path::PathBuf;
use std::{env, sync::LazyLock};

use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        GotoDefinitionParams, GotoDefinitionResponse, Location, PartialResultParams, Position,
        Range, TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

static HOME_DIR: LazyLock<PathBuf> =
    LazyLock::new(|| dirs::home_dir().expect("cannot get home dir"));

#[tokio::test(flavor = "multi_thread")]
async fn gtd_simple() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/app/src/main/groovy/com/example/app/Application.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(6, 35),
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_static_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(47, 37),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy",
        )))
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_this_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(51, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_this_super_member() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(54, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy",
        )))
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_instance_member_access() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(63, 49),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_resolve_chain() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(69, 44),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessResult.groovy",
        )))
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
            position: Position::new(71, 54),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessResult.groovy",
        )))
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

#[tokio::test(flavor = "multi_thread")]
async fn gtd_method_overloading() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(74, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 28,
                character: 17,
            },
            end: Position {
                line: 28,
                character: 32,
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
            position: Position::new(76, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 32,
                character: 17,
            },
            end: Position {
                line: 32,
                character: 32,
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
            position: Position::new(79, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 36,
                character: 17,
            },
            end: Position {
                line: 36,
                character: 32,
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
            position: Position::new(81, 14),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 40,
                character: 17,
            },
            end: Position {
                line: 40,
                character: 32,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test(flavor = "multi_thread")]
async fn gtd_goto_superclass() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(6, 30),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 4,
                character: 15,
            },
            end: Position {
                line: 4,
                character: 26,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test(flavor = "multi_thread")]
async fn gtd_goto_interface() {
    let server = get_test_server("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(6, 53),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            HOME_DIR.join("Projects/lspintar-ws/lspintar/server/tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy",
        )))
        .unwrap(),
        Range {
            start: Position {
                line: 4,
                character: 10,
            },
            end: Position {
                line: 4,
                character: 23,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test(flavor = "multi_thread")]
async fn gtd_goto_property() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(27, 29),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(root.join(
            "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy",
        ))
        .unwrap(),
        Range {
            start: Position {
                line: 10,
                character: 16,
            },
            end: Position {
                line: 10,
                character: 16,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test(flavor = "multi_thread")]
async fn gtd_goto_data_class_field() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(42, 32),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.goto_definition(params).await.unwrap();
    assert!(result.is_some());

    let location = Location::new(
        Url::from_file_path(
            root.join("tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt"),
        )
        .unwrap(),
        Range {
            start: Position {
                line: 2,
                character: 34,
            },
            end: Position {
                line: 2,
                character: 38,
            },
        },
    );

    assert_eq!(result.unwrap(), GotoDefinitionResponse::from(location));
}

#[tokio::test(flavor = "multi_thread")]
async fn gtd_resolve_chain_external() {
    let server = get_test_server("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(24, 36),
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

    // NOTE: for practical reasons, decompiled classes don't return precise
    // symbol locations.
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 0);
}
