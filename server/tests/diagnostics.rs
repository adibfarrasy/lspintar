// Integration tests for server-level semantic diagnostics.
//
// Each test opens a source file (or synthesises content via did_open) and calls
// compute_diagnostics to verify the expected diagnostic codes are (or are not)
// present.  The polyglot-spring fixture is used so that the full project index
// (including BaseRepository, UserRepository, etc.) is available.
//
// Triggering a diagnostic:
//   unimplemented_abstract_methods – remove an `override fun` from a class that
//     implements an interface whose methods are indexed in the project.

use tower_lsp::{
    LanguageServer,
    lsp_types::{DidOpenTextDocumentParams, TextDocumentItem, Url},
};

use crate::util::get_test_server;

mod util;

fn has_code(diags: &[tower_lsp::lsp_types::Diagnostic], code: &str) -> bool {
    diags.iter().any(|d| {
        d.code == Some(tower_lsp::lsp_types::NumberOrString::String(code.to_string()))
    })
}

/// A complete UserRepository that implements both required methods must produce
/// no unimplemented_abstract_methods diagnostic.
#[tokio::test]
async fn no_diagnostic_when_all_methods_implemented() {
    let server = get_test_server("polyglot-spring").await;

    // Use a synthetic URI with .kt extension so this test's did_open doesn't race
    // with diagnostic_when_override_method_deleted on the same documents key.
    let uri = Url::parse("file:///tmp/UserRepositoryComplete.kt").unwrap();

    let content = r#"package com.example

import org.springframework.stereotype.Repository

@Repository
class UserRepository : BaseRepository<User> {
    override fun findById(id: Long): User {
        return User(id, "User $id")
    }

    override fun save(entity: User) {
        println("Saving: ${entity.name}")
    }
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        !has_code(&diags, "unimplemented_abstract_methods"),
        "expected no unimplemented_abstract_methods when all methods are present, got: {diags:?}"
    );
}

/// Removing `override fun findById` from UserRepository must trigger
/// unimplemented_abstract_methods because findById is required by BaseRepository.
#[tokio::test]
async fn diagnostic_when_override_method_deleted() {
    let server = get_test_server("polyglot-spring").await;

    // Use a synthetic URI so this test doesn't race with no_diagnostic_when_all_methods_implemented
    // which opens the real UserRepository.kt via did_open.
    let uri = Url::parse("file:///tmp/UserRepositoryMissingFindById.kt").unwrap();

    // Content with findById removed.
    let content = r#"package com.example

import org.springframework.stereotype.Repository

@Repository
class UserRepository : BaseRepository<User> {
    override fun save(entity: User) {
        println("Saving: ${entity.name}")
    }
}
"#;

    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 2,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        has_code(&diags, "unimplemented_abstract_methods"),
        "expected unimplemented_abstract_methods when findById is missing, got: {diags:?}"
    );

    let msg = diags
        .iter()
        .find(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "unimplemented_abstract_methods".to_string(),
                ))
        })
        .map(|d| d.message.clone())
        .unwrap_or_default();
    assert!(
        msg.contains("findById"),
        "diagnostic message should mention 'findById', got: {msg}"
    );
}

/// A class that implements only one of two same-named overloads must still
/// trigger unimplemented_abstract_methods for the missing overload, even though
/// the *name* is technically present.  Notifier declares both
/// `notify(String)` and `notify(String, int)`; this fixture only supplies the
/// single-arg version.
#[tokio::test]
async fn diagnostic_when_overload_missing() {
    let server = get_test_server("polyglot-spring").await;

    let uri = Url::parse("file:///tmp/PartialNotifier.kt").unwrap();
    let content = r#"package com.example

class PartialNotifier : Notifier {
    override fun notify(message: String) {}
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        has_code(&diags, "unimplemented_abstract_methods"),
        "expected unimplemented_abstract_methods when notify(String, int) overload is missing, got: {diags:?}"
    );

    let msg = diags
        .iter()
        .find(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "unimplemented_abstract_methods".to_string(),
                ))
        })
        .map(|d| d.message.clone())
        .unwrap_or_default();
    assert!(
        msg.contains("notify"),
        "diagnostic message should mention 'notify', got: {msg}"
    );
    assert!(
        msg.contains("int") || msg.contains("Int"),
        "diagnostic message should reference the missing overload's int parameter, got: {msg}"
    );
}

/// The complement: implementing both overloads must produce no diagnostic.
#[tokio::test]
async fn no_diagnostic_when_all_overloads_implemented() {
    let server = get_test_server("polyglot-spring").await;

    let uri = Url::parse("file:///tmp/FullNotifier.kt").unwrap();
    let content = r#"package com.example

class FullNotifier : Notifier {
    override fun notify(message: String) {}
    override fun notify(message: String, priority: Int) {}
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        !has_code(&diags, "unimplemented_abstract_methods"),
        "expected no unimplemented_abstract_methods when both overloads are implemented, got: {diags:?}"
    );
}

// ========================================================================
// syntax_error  — produced by ts_helper for any TS parse error
// ========================================================================

/// Source with an unbalanced brace must surface a `syntax_error` diagnostic.
#[tokio::test]
async fn syntax_error_on_unbalanced_brace() {
    let server = get_test_server("polyglot-spring").await;
    let uri = Url::parse("file:///tmp/SyntaxBroken.kt").unwrap();

    // Missing closing brace.
    let content = r#"package com.example

class SyntaxBroken {
    fun work() {
        if (true) {
            println("oops")
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        has_code(&diags, "syntax_error"),
        "expected syntax_error for unbalanced brace, got: {diags:?}"
    );
}

/// Well-formed source must not produce a syntax_error diagnostic.
#[tokio::test]
async fn no_syntax_error_for_valid_source() {
    let server = get_test_server("polyglot-spring").await;
    let uri = Url::parse("file:///tmp/SyntaxOk.kt").unwrap();

    let content = r#"package com.example

class SyntaxOk {
    fun work(): String {
        return "ok"
    }
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        !has_code(&diags, "syntax_error"),
        "expected no syntax_error for valid source, got: {diags:?}"
    );
}

// ========================================================================
// final_class_extended — extending a class declared `final`
// ========================================================================

/// Extending a final Kotlin class (default in Kotlin — every class is final
/// unless marked `open`) must trigger `final_class_extended`.
#[tokio::test]
async fn final_class_extended_for_default_kotlin_class() {
    let server = get_test_server("polyglot-spring").await;
    let uri = Url::parse("file:///tmp/ExtendsFinal.kt").unwrap();

    // KotlinService is declared `class KotlinService` (not `open`), so the
    // fixture's KotlinService is a final class.
    let content = r#"package com.example

class ExtendsFinal : KotlinService() {
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(
        has_code(&diags, "final_class_extended"),
        "expected final_class_extended when extending a non-open Kotlin class, got: {diags:?}"
    );
}

// ========================================================================
// Diagnostic source metadata
// ========================================================================

/// Every diagnostic the server emits must declare `source = "lspintar"` so
/// editors can identify the producer of the message.
#[tokio::test]
async fn every_diagnostic_has_lspintar_source() {
    let server = get_test_server("polyglot-spring").await;
    let uri = Url::parse("file:///tmp/SourceCheck.kt").unwrap();

    // Use the existing override-missing repro from earlier in this file —
    // we just want at least one diagnostic to inspect.
    let content = r#"package com.example

class SourceCheck : BaseRepository<User> {
    override fun save(entity: User) {}
}
"#;
    server
        .backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "kotlin".to_string(),
                version: 1,
                text: content.to_string(),
            },
        })
        .await;

    let diags = server
        .backend
        .compute_diagnostics(&uri)
        .await
        .expect("compute_diagnostics returned None");

    assert!(!diags.is_empty(), "this fixture must produce at least one diagnostic");
    for d in &diags {
        assert_eq!(
            d.source.as_deref(),
            Some("lspintar"),
            "diagnostic missing or wrong source: {d:?}"
        );
    }
}
