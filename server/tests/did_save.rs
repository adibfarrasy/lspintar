use std::{env, fs};

use tower_lsp::{
    LanguageServer,
    lsp_types::{DidSaveTextDocumentParams, TextDocumentIdentifier, Url},
};

use crate::util::get_test_server;

mod util;

#[tokio::test]
async fn did_save_reindexes_file() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir().expect("cannot get current dir");
    let file_path = root.join(
        "tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/ControllerCopy.groovy",
    );
    let uri = Url::from_file_path(&file_path).expect("cannot parse URI");

    let content = r#"
        package com.example.demo

        class ControllerCopy {
            def testDidSaveMethod() { return 'test' }
        }
    "#;
    fs::write(&file_path, content).expect("cannot write fixture");

    server
        .backend
        .did_save(DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
            text: None,
        })
        .await;

    let repo = server.backend.repo.get().unwrap();
    let symbols = repo
        .find_symbols_by_fqn("com.example.demo.ControllerCopy#testDidSaveMethod")
        .await
        .unwrap();

    fs::remove_file(&file_path).expect("cannot remove fixture");

    assert!(
        !symbols.is_empty(),
        "new method should be indexed after save"
    );
}
