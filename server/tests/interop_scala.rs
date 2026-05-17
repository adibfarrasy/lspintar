//! Scala interop integration tests.
//!
//! Mirrors `interop.rs` but exercises the scala<->java/kotlin/groovy
//! direction now that ScalaService.scala and ScalaConsumer.scala live in
//! the polyglot-spring fixture.

use tower_lsp::{
    LanguageServer,
    lsp_types::{
        GotoDefinitionParams, GotoDefinitionResponse, Location, PartialResultParams, Position,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

const SCALA_CONSUMER: &str = "src/main/scala/com/example/demo/ScalaConsumer.scala";

fn goto_def(uri: Url, line: u32, col: u32) -> GotoDefinitionParams {
    GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position::new(line, col),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    }
}

fn first_location(resp: GotoDefinitionResponse) -> Location {
    match resp {
        GotoDefinitionResponse::Scalar(loc) => loc,
        GotoDefinitionResponse::Array(mut v) => {
            assert!(!v.is_empty(), "goto-def returned empty Array");
            v.remove(0)
        }
        GotoDefinitionResponse::Link(mut v) => {
            assert!(!v.is_empty(), "goto-def returned empty Link list");
            let link = v.remove(0);
            Location::new(link.target_uri, link.target_range)
        }
    }
}

/// Scala consumer -> Java service: cursor on `JavaService` type in
/// `var javaService: JavaService = _`.
#[tokio::test]
async fn interop_gtd_scala_to_java_class() {
    let server = get_test_server("polyglot-spring").await;
    // Line 3:                          ↓ JavaService starts at col 17
    //     "  var javaService: JavaService = _"
    let params = goto_def(server.uri(SCALA_CONSUMER), 3, 22);
    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("scala -> java goto-def returned None");
    let loc = first_location(result);
    assert!(
        loc.uri.path().ends_with("JavaService.java"),
        "expected JavaService.java target, got {}",
        loc.uri
    );
}

/// Scala consumer -> Kotlin service: cursor on `KotlinService` type.
#[tokio::test]
async fn interop_gtd_scala_to_kotlin_class() {
    let server = get_test_server("polyglot-spring").await;
    // Line 4:  "  var kotlinService: KotlinService = _"
    let params = goto_def(server.uri(SCALA_CONSUMER), 4, 25);
    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("scala -> kotlin goto-def returned None");
    let loc = first_location(result);
    assert!(
        loc.uri.path().ends_with("KotlinService.kt"),
        "expected KotlinService.kt target, got {}",
        loc.uri
    );
}

/// Scala consumer -> Groovy service: cursor on `GroovyService` type.
#[tokio::test]
async fn interop_gtd_scala_to_groovy_class() {
    let server = get_test_server("polyglot-spring").await;
    // Line 5:  "  var groovyService: GroovyService = _"
    let params = goto_def(server.uri(SCALA_CONSUMER), 5, 25);
    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("scala -> groovy goto-def returned None");
    let loc = first_location(result);
    assert!(
        loc.uri.path().ends_with("GroovyService.groovy"),
        "expected GroovyService.groovy target, got {}",
        loc.uri
    );
}

/// Indexer smoke: the scala service is picked up by the workspace index
/// and exposed as a class symbol named `ScalaService`.
#[tokio::test]
async fn scala_service_is_indexed() {
    let server = get_test_server("polyglot-spring").await;
    let repo = server.backend.repo.get().expect("repo not initialised");
    let sym = repo
        .find_symbol_by_fqn("com.example.ScalaService")
        .await
        .expect("repo lookup failed");
    let s = sym.expect("ScalaService symbol not found in index");
    assert_eq!(s.short_name, "ScalaService");
    assert_eq!(s.file_type, "scala");
    assert_eq!(s.symbol_type, "Class");
}
