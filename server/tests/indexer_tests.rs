use groovy::GroovySupport;
use lsp_core::vcs::get_vcs_handler;
use pretty_assertions::assert_eq;
use server::{
    Indexer, Repository,
    models::symbol::{Symbol, SymbolMetadata, SymbolParameter},
};
use sqlx::types::Json;
use std::{path::Path, sync::Arc};

#[tokio::test]
async fn test_index_groovy_class() {
    let repo = Arc::new(Repository::new(":memory:").await.unwrap());
    let path =
        Path::new("tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), vcs);
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.index_file(&path).await.expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.User")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "User".to_string(),
            fully_qualified_name: "com.example.User".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec![]),
            line_start: 2,
            line_end: 10,
            char_start: 0,
            char_end: 1,
            ident_line_start: 3,
            ident_line_end: 3,
            ident_char_start: 6,
            ident_char_end: 10,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec!["CompileStatic".to_string()])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_gradle_single_workspace() {
    let repo = Arc::new(Repository::new(":memory:").await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), vcs);
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.UserService")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "UserService".to_string(),
            fully_qualified_name: "com.example.UserService".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec![]),
            line_start: 4,
            line_end: 20,
            char_start: 0,
            char_end: 1,
            ident_line_start: 4,
            ident_line_end: 4,
            ident_char_start: 6,
            ident_char_end: 17,
            extends_name: Some("BaseService".to_string()),
            implements_names: Json(vec!["Serializable".to_string()]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.Repository")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "Repository".to_string(),
            fully_qualified_name: "com.example.Repository".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/Repository.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Interface".to_string(),
            modifiers: Json(vec![]),
            line_start: 2,
            line_end: 8,
            char_start: 0,
            char_end: 1,
            ident_line_start: 6,
            ident_line_end: 6,
            ident_char_start: 10,
            ident_char_end: 20,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: Some("/**\n* lorem ipsum\n* dolor sit amet\n*/".to_string()),
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.User.getDisplayName")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "getDisplayName".to_string(),
            fully_qualified_name: "com.example.User.getDisplayName".to_string(),
            parent_name: Some("com.example.User".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Function".to_string(),
            modifiers: Json(vec![]),
            line_start: 7,
            line_end: 9,
            char_start: 4,
            char_end: 5,
            ident_line_start: 7,
            ident_line_end: 7,
            ident_char_start: 11,
            ident_char_end: 25,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: Some(vec![]),
                return_type: Some("String".to_string()),
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.UserService.userVariable")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "userVariable".to_string(),
            fully_qualified_name: "com.example.UserService.userVariable".to_string(),
            parent_name: Some("com.example.UserService".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Field".to_string(),
            modifiers: Json(vec!["private".to_string()]),
            line_start: 7,
            line_end: 9,
            char_start: 4,
            char_end: 31,
            ident_line_start: 9,
            ident_line_end: 9,
            ident_char_start: 19,
            ident_char_end: 31,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: Some("String".to_string()),
                documentation: None,
                annotations: Some(vec!["Getter".to_string(), "Setter".to_string()])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_gradle_multi_workspace() {
    let repo = Arc::new(Repository::new(":memory:").await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), vcs);
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.core.BaseService")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "BaseService".to_string(),
            fully_qualified_name: "com.example.core.BaseService".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["abstract".to_string()]),
            line_start: 4,
            line_end: 14,
            char_start: 0,
            char_end: 1,
            ident_line_start: 4,
            ident_line_end: 4,
            ident_char_start: 15,
            ident_char_end: 26,
            extends_name: None,
            implements_names: Json(vec!["Serializable".to_string()]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.api.UserController.execute")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "execute".to_string(),
            fully_qualified_name: "com.example.api.UserController.execute".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Function".to_string(),
            modifiers: Json(vec![]),
            line_start: 13,
            line_end: 20,
            char_start: 4,
            char_end: 5,
            ident_line_start: 18,
            ident_line_end: 18,
            ident_char_start: 9,
            ident_char_end: 16,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: Some(vec![]),
                return_type: None,
                documentation: Some("/**\n    * lorem ipsum\n    * dolor sit amet\n    */".to_string()),
                annotations: Some(vec!["Override".to_string()])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.api.UserController$ApiResponse")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "ApiResponse".to_string(),
            fully_qualified_name: "com.example.api.UserController$ApiResponse".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["private".to_string(), "static".to_string()]),
            line_start: 7,
            line_end: 11,
            char_start: 4,
            char_end: 5,
            ident_line_start: 7,
            ident_line_end: 7,
            ident_char_start: 25,
            ident_char_end: 36,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.core.DataProcessor.MAX_BATCH_SIZE")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "Symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;

    assert_eq!(
        symbol,
        Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "MAX_BATCH_SIZE".to_string(),
            fully_qualified_name: "com.example.core.DataProcessor.MAX_BATCH_SIZE".to_string(),
            parent_name: Some("com.example.core.DataProcessor".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Field".to_string(),
            modifiers: Json(vec!["static".to_string(), "final".to_string()]),
            line_start: 5,
            line_end: 5,
            char_start: 4,
            char_end: 42,
            ident_line_start: 5,
            ident_line_end: 5,
            ident_char_start: 21,
            ident_char_end: 35,
            extends_name: None,
            implements_names: Json(vec![]),
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: Some("int".to_string()),
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}
