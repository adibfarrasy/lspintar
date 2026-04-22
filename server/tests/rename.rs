//! Integration tests for `textDocument/rename` across Java, Groovy and Kotlin.

use std::env;

use tower_lsp::{
    LanguageServer,
    lsp_types::{
        PartialResultParams, Position, Range, RenameParams, TextDocumentIdentifier,
        TextDocumentPositionParams, Url, WorkDoneProgressParams, WorkspaceEdit,
    },
};

use crate::util::get_test_server;

mod util;

fn rename_params(path: std::path::PathBuf, position: Position, new_name: &str) -> RenameParams {
    RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(path).expect("bad path"),
            },
            position,
        },
        new_name: new_name.to_string(),
        work_done_progress_params: WorkDoneProgressParams::default(),
    }
}

fn edits_for<'a>(
    edit: &'a WorkspaceEdit,
    file: &std::path::Path,
) -> Option<&'a Vec<tower_lsp::lsp_types::TextEdit>> {
    let uri = Url::from_file_path(file).expect("bad path");
    edit.changes.as_ref()?.get(&uri)
}

// --------------------------------------------------------------------------
// Class / interface rename
// --------------------------------------------------------------------------

/// Renaming the Java class `JavaService` must rename the declaration and
/// every reference to it across the workspace (Groovy `Controller.groovy`
/// uses it as a field type).
#[tokio::test]
async fn rename_java_class_across_files() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let java_service =
        root.join("tests/fixtures/polyglot-spring/src/main/java/com/example/demo/JavaService.java");
    let controller = root
        .join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy");

    // `public class JavaService` — JavaService identifier at (line 5, col 13).
    let params = rename_params(java_service.clone(), Position::new(5, 13), "JavaServiceRenamed");
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let decl_edits = edits_for(&edit, &java_service).expect("declaration file edits");
    assert!(
        decl_edits
            .iter()
            .any(|e| e.new_text == "JavaServiceRenamed"),
        "declaration file must contain a rename edit"
    );

    let controller_edits = edits_for(&edit, &controller);
    assert!(
        controller_edits.is_some() && !controller_edits.unwrap().is_empty(),
        "Controller.groovy should contain edits for JavaService usages"
    );
    for e in controller_edits.unwrap() {
        assert_eq!(e.new_text, "JavaServiceRenamed");
    }
}

/// Renaming the Kotlin class `KotlinService` must rename the declaration and
/// every reference to it across the workspace.
#[tokio::test]
async fn rename_kotlin_class_across_files() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let kotlin_service = root.join(
        "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/KotlinService.kt",
    );
    let controller = root
        .join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy");

    // `class KotlinService` — KotlinService identifier at (line 5, col 6).
    let params = rename_params(
        kotlin_service.clone(),
        Position::new(5, 6),
        "KotlinServiceRenamed",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    assert!(
        edits_for(&edit, &kotlin_service).is_some(),
        "declaration file must be edited"
    );
    assert!(
        edits_for(&edit, &controller).is_some(),
        "Controller.groovy must reference KotlinService"
    );
}

/// Renaming the Groovy class `GroovyService` must rename declaration and
/// references across the workspace.
#[tokio::test]
async fn rename_groovy_class_across_files() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let groovy_service = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/GroovyService.groovy",
    );
    let controller = root
        .join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy");

    // `class GroovyService` — GroovyService identifier at (line 7, col 6).
    let params = rename_params(
        groovy_service.clone(),
        Position::new(7, 6),
        "GroovyServiceRenamed",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    assert!(edits_for(&edit, &groovy_service).is_some());
    assert!(edits_for(&edit, &controller).is_some());
}

// --------------------------------------------------------------------------
// Function rename — signature-matched hierarchy walk
// --------------------------------------------------------------------------

/// Renaming `findById` on the Java interface `BaseRepository` must also
/// rename the Kotlin `UserRepository.findById` override and every call site
/// across the workspace.
#[tokio::test]
async fn rename_function_propagates_to_overrides() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let base = root.join(
        "tests/fixtures/polyglot-spring/src/main/java/com/example/demo/BaseRepository.java",
    );
    let user_repo = root.join(
        "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/UserRepository.kt",
    );

    // `T findById(Long id);` — `findById` identifier at (line 3, col 6).
    let params = rename_params(base.clone(), Position::new(3, 6), "findByIdentifier");
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    assert!(edits_for(&edit, &base).is_some(), "interface declaration edited");
    assert!(
        edits_for(&edit, &user_repo).is_some(),
        "Kotlin override should be renamed"
    );
}

// --------------------------------------------------------------------------
// Local variable rename — scope-aware, shadow-aware
// --------------------------------------------------------------------------

/// Renaming the local `input` in `demoGoToDefinition` only affects occurrences
/// in that method's scope and must be a single-file edit.
#[tokio::test]
async fn rename_local_is_single_file() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let controller = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy",
    );

    // `String input = StringUtils.capitalize("input")` — `input` on LHS at
    // roughly line 25 col 15.  Find it by scanning.
    let content = std::fs::read_to_string(&controller).expect("read controller");
    let (line, col) = find_first_occurrence(&content, "input", Some("String "))
        .expect("locate 'String input' declaration");

    let params = rename_params(
        controller.clone(),
        Position::new(line as u32, col as u32),
        "renamedInput",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let changes = edit.changes.as_ref().expect("has changes");
    assert_eq!(
        changes.len(),
        1,
        "local rename must not touch other files, got {:?}",
        changes.keys().collect::<Vec<_>>()
    );

    let edits = edits_for(&edit, &controller).unwrap();
    assert!(
        edits.len() >= 2,
        "expected declaration + at least one use, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "renamedInput");
    }
}

// --------------------------------------------------------------------------
// Invalid identifier rejection
// --------------------------------------------------------------------------

/// Renaming to a reserved keyword for the target language must be rejected
/// with an error — no edits produced.
#[tokio::test]
async fn rename_rejects_reserved_keyword() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let java_service =
        root.join("tests/fixtures/polyglot-spring/src/main/java/com/example/demo/JavaService.java");

    let params = rename_params(java_service, Position::new(5, 13), "class");
    let result = server.backend.rename(params).await;
    assert!(result.is_err(), "renaming to 'class' must be rejected");
}

/// A name that is not a syntactically valid identifier (starts with digit,
/// contains space) must be rejected.
#[tokio::test]
async fn rename_rejects_syntactically_invalid_name() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cwd");
    let java_service =
        root.join("tests/fixtures/polyglot-spring/src/main/java/com/example/demo/JavaService.java");

    let params = rename_params(java_service, Position::new(5, 13), "1BadName");
    assert!(server.backend.rename(params).await.is_err());
}

// --------------------------------------------------------------------------
// Local-binding variants: parameters, for-each, catch, lambda
// --------------------------------------------------------------------------

/// Renaming a Java method parameter must rename the parameter declaration
/// and every identifier reference to it inside the method body — and must
/// leave the enclosing class alone.
#[tokio::test]
async fn rename_java_method_parameter() {
    let content = r#"package com.example.pkg;

public class ParamHolder {
    public String greet(String name) {
        String prefix = "Hello, ";
        return prefix + name + " (" + name.length() + ")";
    }
}
"#;
    let (path, _file) = write_temp(content, "java");
    let server = get_test_server("polyglot-spring").await;

    // `String name` parameter — `name` identifier at line 3, col 32.
    let (line, col) =
        find_byte_position(content, "greet(String name)", "name").expect("locate param");
    let params = rename_params(
        path.clone(),
        Position::new(line as u32, col as u32),
        "who",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let edits = edits_for(&edit, &path).expect("edits for file");
    assert!(
        edits.len() >= 3,
        "expected declaration + 2 uses, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "who");
    }
    assert_eq!(
        edit.changes.as_ref().unwrap().len(),
        1,
        "single-file rename"
    );
}

/// Renaming a Java enhanced-for binding must rename the binding and every
/// use in the loop body, leaving the iterated collection alone.
#[tokio::test]
async fn rename_java_for_each_binding() {
    let content = r#"package com.example.pkg;

import java.util.List;

public class ForHolder {
    public int total(List<Integer> items) {
        int sum = 0;
        for (Integer item : items) {
            sum += item;
        }
        return sum;
    }
}
"#;
    let (path, _file) = write_temp(content, "java");
    let server = get_test_server("polyglot-spring").await;

    let (line, col) =
        find_byte_position(content, "Integer item :", "item").expect("locate for binding");
    let params = rename_params(
        path.clone(),
        Position::new(line as u32, col as u32),
        "entry",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let edits = edits_for(&edit, &path).expect("edits for file");
    assert!(
        edits.len() >= 2,
        "expected binding + at least one use, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "entry");
    }
}

/// Renaming a Java catch-clause parameter must rename the parameter and its
/// uses inside the catch block.
#[tokio::test]
async fn rename_java_catch_parameter() {
    let content = r#"package com.example.pkg;

public class CatchHolder {
    public String safeRun(Runnable r) {
        try {
            r.run();
            return "ok";
        } catch (RuntimeException ex) {
            return ex.getMessage();
        }
    }
}
"#;
    let (path, _file) = write_temp(content, "java");
    let server = get_test_server("polyglot-spring").await;

    let (line, col) =
        find_byte_position(content, "RuntimeException ex", "ex").expect("locate catch param");
    let params = rename_params(
        path.clone(),
        Position::new(line as u32, col as u32),
        "err",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let edits = edits_for(&edit, &path).expect("edits for file");
    assert!(
        edits.len() >= 2,
        "expected param decl + use, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "err");
    }
}

/// Renaming an explicit Groovy closure parameter must rename the parameter
/// and every reference inside the closure body, leaving outer scopes alone.
#[tokio::test]
async fn rename_groovy_closure_parameter() {
    let content = r#"package com.example.pkg

class ClosureHolder {
    List<String> shout(List<String> words) {
        return words.collect { word -> word.toUpperCase() + '!' }
    }
}
"#;
    let (path, _file) = write_temp(content, "groovy");
    let server = get_test_server("polyglot-spring").await;

    let (line, col) =
        find_byte_position(content, "{ word ->", "word").expect("locate closure param");
    let params = rename_params(
        path.clone(),
        Position::new(line as u32, col as u32),
        "token",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let edits = edits_for(&edit, &path).expect("edits for file");
    assert!(
        edits.len() >= 2,
        "expected param decl + at least one use, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "token");
    }
}

/// Renaming an explicit Kotlin lambda parameter must rename the parameter
/// and every reference inside the lambda body, leaving outer scopes alone.
#[tokio::test]
async fn rename_kotlin_lambda_parameter() {
    let content = r#"package com.example.pkg

class LambdaHolder {
    fun shout(words: List<String>): List<String> =
        words.map { word -> word.uppercase() + "!" }
}
"#;
    let (path, _file) = write_temp(content, "kt");
    let server = get_test_server("polyglot-spring").await;

    let (line, col) =
        find_byte_position(content, "{ word ->", "word").expect("locate lambda param");
    let params = rename_params(
        path.clone(),
        Position::new(line as u32, col as u32),
        "token",
    );
    let edit = server
        .backend
        .rename(params)
        .await
        .expect("rename Ok")
        .expect("WorkspaceEdit returned");

    let edits = edits_for(&edit, &path).expect("edits for file");
    assert!(
        edits.len() >= 2,
        "expected param decl + at least one use, got {}",
        edits.len()
    );
    for e in edits {
        assert_eq!(e.new_text, "token");
    }
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

/// Returns the (line, column) of the first occurrence of `needle` in
/// `content`.  When `following_prefix` is provided, requires that the text
/// immediately before the match equals the prefix (byte-for-byte).
fn find_first_occurrence(
    content: &str,
    needle: &str,
    following_prefix: Option<&str>,
) -> Option<(usize, usize)> {
    for (idx, line) in content.lines().enumerate() {
        if let Some(prefix) = following_prefix {
            if let Some(p) = line.find(prefix) {
                let candidate = p + prefix.len();
                if line[candidate..].starts_with(needle) {
                    return Some((idx, candidate));
                }
            }
        } else if let Some(p) = line.find(needle) {
            return Some((idx, p));
        }
    }
    None
}

/// Sanity helper for test debugging — not part of an assertion.
#[allow(dead_code)]
fn assert_range_not_empty(r: Range) {
    assert!(
        r.start != r.end,
        "expected non-empty range, got {:?}",
        r
    );
}

/// Write `content` to a uniquely-named temp file with the given extension
/// and return its path.  The returned `NamedTempFile` must be kept alive
/// for the duration of the test to prevent cleanup.
fn write_temp(content: &str, ext: &str) -> (std::path::PathBuf, tempfile::NamedTempFile) {
    let file = tempfile::Builder::new()
        .suffix(&format!(".{}", ext))
        .tempfile()
        .expect("create tempfile");
    std::fs::write(file.path(), content).expect("write temp content");
    (file.path().to_path_buf(), file)
}

/// Locate (line, column) of `needle` occurring within the line that
/// contains `line_marker`.  Used to pinpoint parameter/binding positions
/// without hand-counting.
fn find_byte_position(content: &str, line_marker: &str, needle: &str) -> Option<(usize, usize)> {
    for (idx, line) in content.lines().enumerate() {
        if let Some(_) = line.find(line_marker) {
            if let Some(col) = line.find(needle) {
                return Some((idx, col));
            }
        }
    }
    None
}
