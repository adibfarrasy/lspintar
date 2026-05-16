//! Cross-language interop integration tests.
//!
//! Coverage matrix (consumer language → producer language):
//!
//!     java   -> groovy    (JavaConsumer    -> GroovyService)
//!     java   -> kotlin    (JavaConsumer    -> KotlinService)
//!     kotlin -> groovy    (KotlinConsumer  -> GroovyService)
//!     kotlin -> java      (UserRepository  -> BaseRepository)         [existing fixture]
//!     groovy -> java      (Controller      -> JavaService)            [existing fixture]
//!     groovy -> kotlin    (Controller      -> KotlinService, User)    [existing fixture]
//!
//! The first three rows are why JavaConsumer.java and KotlinConsumer.kt
//! exist in the polyglot-spring fixture — without them lspintar's resolver
//! is never exercised against a Java-from-Java import of a Groovy or Kotlin
//! symbol, or a Kotlin import of a Groovy symbol.

use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer,
    lsp_types::{
        GotoDefinitionParams, GotoDefinitionResponse, HoverContents, HoverParams, Location,
        PartialResultParams, Position, Range, ReferenceContext, ReferenceParams, RenameParams,
        TextDocumentIdentifier, TextDocumentPositionParams, Url, WorkDoneProgressParams,
    },
};

use crate::util::get_test_server;

mod util;

// --- fixture paths -------------------------------------------------------

const JAVA_CONSUMER: &str = "src/main/java/com/example/demo/JavaConsumer.java";
const KOTLIN_CONSUMER: &str = "src/main/kotlin/com/example/demo/KotlinConsumer.kt";
const CONTROLLER: &str = "src/main/groovy/com/example/demo/Controller.groovy";

const JAVA_SERVICE: &str = "src/main/java/com/example/demo/JavaService.java";
const KOTLIN_SERVICE: &str = "src/main/kotlin/com/example/demo/KotlinService.kt";
const GROOVY_SERVICE: &str = "src/main/groovy/com/example/demo/GroovyService.groovy";

// --- small request builders ---------------------------------------------

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

fn hover_params(uri: Url, line: u32, col: u32) -> HoverParams {
    HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position::new(line, col),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    }
}

fn ref_params(uri: Url, line: u32, col: u32, include_decl: bool) -> ReferenceParams {
    ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position::new(line, col),
        },
        context: ReferenceContext { include_declaration: include_decl },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
    }
}

fn rename_request(uri: Url, line: u32, col: u32, new_name: &str) -> RenameParams {
    RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position::new(line, col),
        },
        new_name: new_name.to_string(),
        work_done_progress_params: WorkDoneProgressParams::default(),
    }
}

/// Pull the first scalar `Location` out of a goto-definition response.
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

fn hover_value(h: HoverContents) -> String {
    match h {
        HoverContents::Markup(m) => m.value,
        HoverContents::Scalar(s) => format!("{s:?}"),
        HoverContents::Array(_) => panic!("unexpected hover array form"),
    }
}

// ========================================================================
// goto_definition — class-name resolution across language boundaries
// ========================================================================

// java -> groovy: cursor on `GroovyService` field type in JavaConsumer.java.
#[tokio::test]
async fn interop_gtd_java_to_groovy_class() {
    let server = get_test_server("polyglot-spring").await;

    // line 3: `    private GroovyService groovyService;`
    //          0123456789012345
    let params = goto_def(server.uri(JAVA_CONSUMER), 3, 14);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("java -> groovy goto-def returned None");
    let loc = first_location(result);

    let expected = Location::new(
        server.uri(GROOVY_SERVICE),
        Range {
            start: Position { line: 7, character: 6 },
            end: Position { line: 7, character: 19 },
        },
    );
    assert_eq!(loc, expected);
}

// java -> kotlin: cursor on `KotlinService` field type in JavaConsumer.java.
#[tokio::test]
async fn interop_gtd_java_to_kotlin_class() {
    let server = get_test_server("polyglot-spring").await;

    // line 4: `    private KotlinService kotlinService;`
    let params = goto_def(server.uri(JAVA_CONSUMER), 4, 14);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("java -> kotlin goto-def returned None");
    let loc = first_location(result);

    let expected = Location::new(
        server.uri(KOTLIN_SERVICE),
        Range {
            start: Position { line: 5, character: 6 },
            end: Position { line: 5, character: 19 },
        },
    );
    assert_eq!(loc, expected);
}

// kotlin -> groovy: cursor on `GroovyService` type annotation in KotlinConsumer.kt.
#[tokio::test]
async fn interop_gtd_kotlin_to_groovy_class() {
    let server = get_test_server("polyglot-spring").await;

    // line 3 (0-indexed): `    lateinit var groovyService: GroovyService`
    let params = goto_def(server.uri(KOTLIN_CONSUMER), 3, 32);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("kotlin -> groovy goto-def returned None");
    let loc = first_location(result);

    let expected = Location::new(
        server.uri(GROOVY_SERVICE),
        Range {
            start: Position { line: 7, character: 6 },
            end: Position { line: 7, character: 19 },
        },
    );
    assert_eq!(loc, expected);
}

// kotlin -> java: BaseRepository in UserRepository's `: BaseRepository<User>` clause.
// Already covered by rename test; included here for symmetric matrix coverage.
#[tokio::test]
async fn interop_gtd_kotlin_to_java_class() {
    let server = get_test_server("polyglot-spring").await;

    // UserRepository.kt line 5: `class UserRepository : BaseRepository<User> {`
    //                                              0         1         2         3
    //                                              0123456789012345678901234567890123
    let params = goto_def(
        server.uri("src/main/kotlin/com/example/demo/UserRepository.kt"),
        5,
        24,
    );

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("kotlin -> java goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("src/main/java/com/example/demo/BaseRepository.java"))
            .unwrap_or(false),
        "expected jump into BaseRepository.java, got {:?}",
        loc.uri
    );
}

// groovy -> java: re-asserted here so the matrix is self-contained.
#[tokio::test]
async fn interop_gtd_groovy_to_java_class() {
    let server = get_test_server("polyglot-spring").await;

    // Controller.groovy line 11 (0-indexed): `    JavaService javaService`
    let params = goto_def(server.uri(CONTROLLER), 11, 4);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("groovy -> java goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("JavaService.java"))
            .unwrap_or(false),
        "expected jump into JavaService.java, got {:?}",
        loc.uri
    );
}

// groovy -> kotlin: cursor on `KotlinService` field type in Controller.groovy.
#[tokio::test]
async fn interop_gtd_groovy_to_kotlin_class() {
    let server = get_test_server("polyglot-spring").await;

    // Controller.groovy line 17 (0-indexed): `    KotlinService kotlinService`
    let params = goto_def(server.uri(CONTROLLER), 17, 4);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("groovy -> kotlin goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("KotlinService.kt"))
            .unwrap_or(false),
        "expected jump into KotlinService.kt, got {:?}",
        loc.uri
    );
}

// ========================================================================
// goto_definition — method call across language boundaries
// ========================================================================

// java -> groovy method: `groovyService.process(input)` in JavaConsumer.java
#[tokio::test]
async fn interop_gtd_java_to_groovy_method() {
    let server = get_test_server("polyglot-spring").await;

    // line 7: `        return groovyService.process(input);`
    //                  0         1         2         3
    //                  0123456789012345678901234567890123
    let params = goto_def(server.uri(JAVA_CONSUMER), 7, 30);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("java -> groovy method goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("GroovyService.groovy"))
            .unwrap_or(false),
        "expected jump into GroovyService.groovy, got {:?}",
        loc.uri
    );
    assert_eq!(loc.range.start.line, 8, "should land on `process` method declaration line");
}

// java -> kotlin method: `kotlinService.process(input)` in JavaConsumer.java
#[tokio::test]
async fn interop_gtd_java_to_kotlin_method() {
    let server = get_test_server("polyglot-spring").await;

    // line 11: `        return kotlinService.process(input);`
    let params = goto_def(server.uri(JAVA_CONSUMER), 11, 30);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("java -> kotlin method goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("KotlinService.kt"))
            .unwrap_or(false),
        "expected jump into KotlinService.kt, got {:?}",
        loc.uri
    );
    assert_eq!(loc.range.start.line, 6, "should land on `fun process` declaration line");
}

// kotlin -> groovy method: `groovyService.process(input)` in KotlinConsumer.kt
#[tokio::test]
async fn interop_gtd_kotlin_to_groovy_method() {
    let server = get_test_server("polyglot-spring").await;

    // line 5 (0-indexed):
    //   `    fun useGroovy(input: String): String = groovyService.process(input)`
    // Cursor on the `process` call.
    let params = goto_def(server.uri(KOTLIN_CONSUMER), 5, 57);

    let result = server
        .backend
        .goto_definition(params)
        .await
        .unwrap()
        .expect("kotlin -> groovy method goto-def returned None");
    let loc = first_location(result);

    assert!(
        loc.uri
            .to_file_path()
            .map(|p| p.ends_with("GroovyService.groovy"))
            .unwrap_or(false),
        "expected jump into GroovyService.groovy, got {:?}",
        loc.uri
    );
    assert_eq!(loc.range.start.line, 8);
}

// ========================================================================
// hover — language tag in the markdown matches the producer language
// ========================================================================

// java consumer hovering over a groovy symbol -> markdown should be tagged ```groovy
#[tokio::test]
async fn interop_hover_java_over_groovy_class() {
    let server = get_test_server("polyglot-spring").await;
    let params = hover_params(server.uri(JAVA_CONSUMER), 3, 14);

    let hover = server
        .backend
        .hover(params)
        .await
        .unwrap()
        .expect("hover returned None");
    let value = hover_value(hover.contents);
    assert!(
        value.starts_with("```groovy"),
        "hover over Groovy producer should be a groovy code-fence, got: {value}"
    );
    assert!(value.contains("class GroovyService"));
}

// kotlin consumer hovering over a groovy symbol
#[tokio::test]
async fn interop_hover_kotlin_over_groovy_class() {
    let server = get_test_server("polyglot-spring").await;
    let params = hover_params(server.uri(KOTLIN_CONSUMER), 3, 32);

    let hover = server
        .backend
        .hover(params)
        .await
        .unwrap()
        .expect("hover returned None");
    let value = hover_value(hover.contents);
    assert!(
        value.starts_with("```groovy"),
        "hover should still surface groovy markdown when called from kotlin, got: {value}"
    );
    assert!(value.contains("class GroovyService"));
}

// groovy consumer hovering over a java symbol -> ```java fence (already partially
// covered by hover_class but kept here for the matrix).
#[tokio::test]
async fn interop_hover_groovy_over_java_class() {
    let server = get_test_server("polyglot-spring").await;
    // Controller.groovy line 11, col 5 -> `JavaService` (from existing hover_class).
    let params = hover_params(server.uri(CONTROLLER), 11, 5);

    let hover = server
        .backend
        .hover(params)
        .await
        .unwrap()
        .expect("hover returned None");
    let value = hover_value(hover.contents);
    assert!(value.starts_with("```java"));
    assert!(value.contains("class JavaService"));
}

// ========================================================================
// references — cross-language usage discovery
// ========================================================================

// References on GroovyService's declaration must include both the Java consumer
// and the Kotlin consumer (in addition to the Groovy Controller).
#[tokio::test]
async fn interop_references_groovy_class_finds_all_consumers() {
    let server = get_test_server("polyglot-spring").await;
    let params = ref_params(server.uri(GROOVY_SERVICE), 7, 6, /* include_decl */ false);

    let result = server
        .backend
        .references(params)
        .await
        .unwrap()
        .expect("references returned None");

    let locations: Vec<_> = result
        .iter()
        .filter_map(|loc| loc.uri.to_file_path().ok())
        .collect();

    let has_java_consumer = locations.iter().any(|p| p.ends_with("JavaConsumer.java"));
    let has_kotlin_consumer = locations.iter().any(|p| p.ends_with("KotlinConsumer.kt"));
    let has_controller = locations.iter().any(|p| p.ends_with("Controller.groovy"));

    assert!(
        has_java_consumer,
        "JavaConsumer.java must appear in cross-language references, got: {:?}",
        locations
    );
    assert!(
        has_kotlin_consumer,
        "KotlinConsumer.kt must appear in cross-language references, got: {:?}",
        locations
    );
    assert!(
        has_controller,
        "Controller.groovy must appear in references, got: {:?}",
        locations
    );
}

// References on KotlinService's declaration must include the Java consumer
// (in addition to the Groovy Controller).
#[tokio::test]
async fn interop_references_kotlin_class_finds_all_consumers() {
    let server = get_test_server("polyglot-spring").await;
    let params = ref_params(server.uri(KOTLIN_SERVICE), 5, 6, /* include_decl */ false);

    let result = server
        .backend
        .references(params)
        .await
        .unwrap()
        .expect("references returned None");

    let locations: Vec<_> = result
        .iter()
        .filter_map(|loc| loc.uri.to_file_path().ok())
        .collect();

    let has_java_consumer = locations.iter().any(|p| p.ends_with("JavaConsumer.java"));
    let has_controller = locations.iter().any(|p| p.ends_with("Controller.groovy"));

    assert!(
        has_java_consumer,
        "JavaConsumer.java must appear in cross-language references, got: {:?}",
        locations
    );
    assert!(
        has_controller,
        "Controller.groovy must appear in references, got: {:?}",
        locations
    );
}

// References on JavaService's declaration must include the Groovy Controller.
#[tokio::test]
async fn interop_references_java_class_finds_groovy_consumer() {
    let server = get_test_server("polyglot-spring").await;
    let params = ref_params(server.uri(JAVA_SERVICE), 5, 13, /* include_decl */ false);

    let result = server
        .backend
        .references(params)
        .await
        .unwrap()
        .expect("references returned None");

    let locations: Vec<_> = result
        .iter()
        .filter_map(|loc| loc.uri.to_file_path().ok())
        .collect();

    assert!(
        locations.iter().any(|p| p.ends_with("Controller.groovy")),
        "Controller.groovy must appear in cross-language references, got: {:?}",
        locations
    );
}

// ========================================================================
// rename — propagation across language boundaries
// ========================================================================

// Renaming GroovyService at its declaration must produce edits in:
//   - GroovyService.groovy (decl)
//   - Controller.groovy   (existing usage)
//   - JavaConsumer.java   (new)
//   - KotlinConsumer.kt   (new)
#[tokio::test]
async fn interop_rename_groovy_class_propagates_to_all_consumers() {
    let server = get_test_server("polyglot-spring").await;
    let params = rename_request(
        server.uri(GROOVY_SERVICE),
        7,
        6,
        "GroovyServiceRenamed",
    );

    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let changes = edit.changes.expect("rename should produce per-file edits");
    let edited_paths: Vec<_> = changes
        .keys()
        .filter_map(|u| u.to_file_path().ok())
        .collect();

    for required in &[
        "GroovyService.groovy",
        "Controller.groovy",
        "JavaConsumer.java",
        "KotlinConsumer.kt",
    ] {
        assert!(
            edited_paths.iter().any(|p| p.ends_with(required)),
            "expected {required} in rename edits, got: {:?}",
            edited_paths
        );
    }
}

// Renaming KotlinService at its declaration must produce edits in the Java
// consumer too.
#[tokio::test]
async fn interop_rename_kotlin_class_propagates_to_java_consumer() {
    let server = get_test_server("polyglot-spring").await;
    let params = rename_request(
        server.uri(KOTLIN_SERVICE),
        5,
        6,
        "KotlinServiceRenamed",
    );

    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let changes = edit.changes.expect("rename should produce per-file edits");
    let edited_paths: Vec<_> = changes
        .keys()
        .filter_map(|u| u.to_file_path().ok())
        .collect();

    assert!(
        edited_paths.iter().any(|p| p.ends_with("JavaConsumer.java")),
        "expected JavaConsumer.java to be edited, got: {:?}",
        edited_paths
    );
    assert!(
        edited_paths.iter().any(|p| p.ends_with("KotlinService.kt")),
        "expected declaration file in edits, got: {:?}",
        edited_paths
    );
}
