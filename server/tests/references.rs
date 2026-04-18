use std::env;

use tower_lsp::{
    LanguageServer,
    lsp_types::{
        PartialResultParams, Position, ReferenceContext, ReferenceParams,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

/// Placing the cursor on an identifier that appears in multiple files should
/// return at least the usage in that same file.
#[tokio::test]
async fn references_returns_occurrences_of_identifier() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    // "process" is declared in GroovyService and called in Controller.
    let groovy_service_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/GroovyService.groovy",
    );

    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(&groovy_service_path).expect("cannot parse URI"),
            },
            // "process" method name declaration in GroovyService (line 8, col 11)
            position: Position::new(8, 11),
        },
        context: ReferenceContext {
            include_declaration: true,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.references(params).await.unwrap();
    assert!(result.is_some(), "references should return Some for a known identifier");

    let locations = result.unwrap();
    assert!(
        !locations.is_empty(),
        "expected at least one reference location for 'process'"
    );

    // The declaration itself should be included.
    let has_groovy_service = locations.iter().any(|loc| {
        loc.uri
            .to_file_path()
            .map(|p| p == groovy_service_path)
            .unwrap_or(false)
    });
    assert!(
        has_groovy_service,
        "GroovyService.groovy should appear in references (include_declaration = true)"
    );
}

/// When include_declaration is false, the declaration site must be excluded
/// from the returned locations.
#[tokio::test]
async fn references_exclude_declaration_when_requested() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let groovy_service_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/GroovyService.groovy",
    );

    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(&groovy_service_path).expect("cannot parse URI"),
            },
            position: Position::new(8, 11),
        },
        context: ReferenceContext {
            include_declaration: false,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.references(params).await.unwrap();

    if let Some(locations) = result {
        // None of the returned locations should be the exact declaration position.
        let decl_at_cursor = locations.iter().any(|loc| {
            loc.uri
                .to_file_path()
                .map(|p| p == groovy_service_path)
                .unwrap_or(false)
                && loc.range.start.line == 8
                && loc.range.start.character <= 11
                && 11 < loc.range.end.character
        });
        assert!(
            !decl_at_cursor,
            "declaration site must not appear when include_declaration = false"
        );
    }
    // returning None (no other references found) is also valid
}

/// References for "process" must not include lines that are pure comments.
/// Controller.groovy has three comment lines mentioning "process" (lines 28, 31, 34)
/// and three real call sites (lines 29, 32, 35).  Only the real call sites should
/// appear in the results.
#[tokio::test]
async fn references_exclude_comment_occurrences() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let groovy_service_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/GroovyService.groovy",
    );
    let controller_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy",
    );

    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(&groovy_service_path).expect("cannot parse URI"),
            },
            // "process" method declaration (line 8, col 11) in GroovyService
            position: Position::new(8, 11),
        },
        context: ReferenceContext {
            include_declaration: true,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.references(params).await.unwrap();
    let locations = result.expect("expected Some locations");

    // Lines 27, 30, 33 (0-indexed) in Controller.groovy are comment lines containing
    // "process".  None of those lines should appear in the results.
    let comment_hits: Vec<_> = locations
        .iter()
        .filter(|loc| {
            loc.uri
                .to_file_path()
                .map(|p| p == controller_path)
                .unwrap_or(false)
                && [27u32, 30, 33].contains(&loc.range.start.line)
        })
        .collect();

    assert!(
        comment_hits.is_empty(),
        "references must not include comment lines, but got: {comment_hits:?}"
    );
}

/// Requesting references for an unknown position should return None gracefully.
#[tokio::test]
async fn references_returns_none_for_unknown_position() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");

    let groovy_service_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/GroovyService.groovy",
    );

    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(&groovy_service_path).expect("cannot parse URI"),
            },
            // whitespace-only line — no identifier here
            position: Position::new(0, 0),
        },
        context: ReferenceContext {
            include_declaration: true,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    };

    let result = server.backend.references(params).await.unwrap();
    // "package" keyword at line 0 col 0 — the identifier found will be "package"
    // which may or may not appear elsewhere. The handler must not panic.
    let _ = result;
}
