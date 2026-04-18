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
