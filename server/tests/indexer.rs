use groovy::GroovySupport;
use java::JavaSupport;
use kotlin::KotlinSupport;
use lsp_core::{
    build_tools::{BuildToolHandler, gradle::GradleHandler},
    vcs::get_vcs_handler,
};
use lspintar_server::{
    Indexer, Repository,
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata},
    },
};
use pretty_assertions::assert_eq;
use sqlx::types::Json;
use std::{path::Path, sync::Arc};
use uuid::Uuid;

#[tokio::test]
async fn test_index_groovy_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path =
        Path::new("tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.index_file(&path).await.expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.User", "NONE")
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
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec![]),
            line_start: 4,
            line_end: 12,
            char_start: 0,
            char_end: 1,
            ident_line_start: 5,
            ident_line_end: 5,
            ident_char_start: 6,
            ident_char_end: 10,
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
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.UserService", "NONE")
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
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.UserService".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec![]),
            line_start: 8,
            line_end: 37,
            char_start: 0,
            char_end: 1,
            ident_line_start: 8,
            ident_line_end: 8,
            ident_char_start: 6,
            ident_char_end: 17,
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
        .find_symbol_by_fqn_and_branch("com.example.Repository", "NONE")
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
            package_name: "com.example".to_string(),
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
        .find_symbol_by_fqn_and_branch("com.example.User#getDisplayName", "NONE")
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
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User#getDisplayName".to_string(),
            parent_name: Some("com.example.User".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Function".to_string(),
            modifiers: Json(vec![]),
            line_start: 9,
            line_end: 11,
            char_start: 4,
            char_end: 5,
            ident_line_start: 9,
            ident_line_end: 9,
            ident_char_start: 11,
            ident_char_end: 25,
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
        .find_symbol_by_fqn_and_branch("com.example.UserService#userVariable", "NONE")
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
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.UserService#userVariable".to_string(),
            parent_name: Some("com.example.UserService".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Field".to_string(),
            modifiers: Json(vec!["private".to_string()]),
            line_start: 11,
            line_end: 11,
            char_start: 4,
            char_end: 31,
            ident_line_start: 11,
            ident_line_end: 11,
            ident_char_start: 19,
            ident_char_end: 31,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: Some("String".to_string()),
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_class_multi_project() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.core.BaseService", "NONE")
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
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.BaseService".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy".to_string(),
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
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_method() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.api.UserController#execute", "NONE")
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
            package_name: "com.example.api".to_string(),
            fully_qualified_name: "com.example.api.UserController#execute".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy".to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Function".to_string(),
            modifiers: Json(vec![]),
            line_start: 14,
            line_end: 21,
            char_start: 4,
            char_end: 5,
            ident_line_start: 19,
            ident_line_end: 19,
            ident_char_start: 9,
            ident_char_end: 16,
            metadata: Json(SymbolMetadata {
                parameters: Some(vec![]),
                return_type: None,
                documentation: Some("/**\n    * lorem ipsum\n    * dolor sit amet\n    */".to_string()),
                annotations: Some(vec!["Override".to_string()])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_nested_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.api.UserController#ApiResponse", "NONE")
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
            package_name: "com.example.api".to_string(),
            fully_qualified_name: "com.example.api.UserController#ApiResponse".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy".to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["private".to_string(), "static".to_string()]),
            line_start: 8,
            line_end: 12,
            char_start: 4,
            char_end: 5,
            ident_line_start: 8,
            ident_line_end: 8,
            ident_char_start: 25,
            ident_char_end: 36,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_groovy_field() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.core.DataProcessor#MAX_BATCH_SIZE", "NONE")
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
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.DataProcessor#MAX_BATCH_SIZE".to_string(),
            parent_name: Some("com.example.core.DataProcessor".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy".to_string(),
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

#[tokio::test]
async fn test_index_groovy_inheritance() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path)
        .await
        .expect("Indexing failed");

    let results = repo
        .find_supers_by_symbol_fqn_and_branch("com.example.api.UserController", "NONE")
        .await
        .expect("Query failed")
        .into_iter()
        .map(|mut symbol| {
            symbol.id = None;
            symbol.last_modified = 0;
            symbol
        })
        .collect::<Vec<Symbol>>();

    assert_eq!(results.len(), 2, "Should find superclass and interface");

    let superclass = &results[0];
    assert_eq!(
        superclass,
        &Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "BaseService".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.BaseService".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy".to_string(),
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
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );

    let super_interface = &results[1];
    assert_eq!(
        super_interface,
        &Symbol {
            id: None,
            vcs_branch: "NONE".to_string(),
            short_name: "DataProcessor".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.DataProcessor".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy".to_string(),
            file_type: "Groovy".to_string(),
            symbol_type: "Interface".to_string(),
            modifiers: Json(vec![]),
            line_start: 4,
            line_end: 9,
            char_start: 0,
            char_end: 1,
            ident_line_start: 4,
            ident_line_end: 4,
            ident_char_start: 10,
            ident_char_end: 23,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_kotlin_data_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("kt", Arc::new(KotlinSupport::new()));
    indexer.index_file(&path).await.expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn_and_branch("com.example.User", "NONE")
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
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path: "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt"
                .to_string(),
            file_type: "Kotlin".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["data".to_string()]),
            line_start: 2,
            line_end: 2,
            char_start: 0,
            char_end: 47,
            ident_line_start: 2,
            ident_line_end: 2,
            ident_char_start: 11,
            ident_char_end: 15,
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
        .find_symbol_by_fqn_and_branch("com.example.User#name", "NONE")
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
            short_name: "name".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User#name".to_string(),
            parent_name: Some("com.example.User".to_string()),
            file_path: "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt"
                .to_string(),
            file_type: "Kotlin".to_string(),
            symbol_type: "Field".to_string(),
            modifiers: Json(vec![]),
            line_start: 2,
            line_end: 2,
            char_start: 30,
            char_end: 46,
            ident_line_start: 2,
            ident_line_end: 2,
            ident_char_start: 34,
            ident_char_end: 38,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: Some("String".to_string()),
                documentation: None,
                annotations: Some(vec![])
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_external_dep_source_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let gradle_handler = GradleHandler;
    let dep_jars = gradle_handler.get_dependency_paths(&path).unwrap();

    let jar_path = dep_jars
        .iter()
        .map(|p| p.clone().1)
        .find(|p| {
            if let Some(source_jar) = p {
                source_jar.to_string_lossy().contains("groovy-json")
                    && source_jar.to_string_lossy().contains("-sources.jar")
            } else {
                false
            }
        })
        .expect("groovy-json sources.jar not found")
        .expect("groovy-json is empty");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_jar(&jar_path)
        .await
        .expect("JAR indexing failed");

    let result = repo
        .find_external_symbol_by_fqn("groovy.json.JsonBuilder")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();

    let doc_string = "/**\n * A builder for creating JSON payloads.\n * <p>\n * This builder supports the usual builder syntax made of nested method calls and closures,\n * but also some specific aspects of JSON data structures, such as list of values, etc.\n * Please make sure to have a look at the various methods provided by this builder\n * to be able to learn about the various possibilities of usage.\n * <p>\n * Example:\n * <pre><code class=\"groovyTestCase\">\n *       def builder = new groovy.json.JsonBuilder()\n *       def root = builder.people {\n *           person {\n *               firstName 'Guillaume'\n *               lastName 'Laforge'\n *               // Named arguments are valid values for objects too\n *               address(\n *                       city: 'Paris',\n *                       country: 'France',\n *                       zip: 12345,\n *               )\n *               married true\n *               // a list of values\n *               conferences 'JavaOne', 'Gr8conf'\n *           }\n *       }\n *\n *       // creates a data structure made of maps (Json object) and lists (Json array)\n *       assert root instanceof Map\n *\n *       assert builder.toString() == '{\"people\":{\"person\":{\"firstName\":\"Guillaume\",\"lastName\":\"Laforge\",\"address\":{\"city\":\"Paris\",\"country\":\"France\",\"zip\":12345},\"married\":true,\"conferences\":[\"JavaOne\",\"Gr8conf\"]}}}'\n * </code></pre>\n *\n * @since 1.8.0\n */";

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "groovy/json/JsonBuilder.java".to_string(),
            short_name: "JsonBuilder".to_string(),
            fully_qualified_name: "groovy.json.JsonBuilder".to_string(),
            package_name: "groovy.json".to_string(),
            parent_name: Some("groovy.json".to_string()),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["public".to_string()]),
            line_start: 35,
            line_end: 421,
            char_start: 0,
            char_end: 1,
            ident_line_start: 70,
            ident_line_end: 70,
            ident_char_start: 13,
            ident_char_end: 24,
            needs_decompilation: false,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: Some(doc_string.to_string()),
                annotations: Some(vec![]),
            },),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_external_dep_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let gradle_handler = GradleHandler;
    let dep_jars = gradle_handler.get_dependency_paths(&path).unwrap();

    let jar_path = dep_jars
        .iter()
        .map(|p| p.clone().0)
        .find(|jar| {
            jar.to_string_lossy().contains("groovy-json")
                && !jar.to_string_lossy().contains("-sources.jar")
        })
        .expect("groovy-json bytecode class jar not found");

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_jar(&jar_path)
        .await
        .expect("JAR indexing failed");

    let result = repo
        .find_external_symbol_by_fqn("groovy.json.JsonBuilder")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "groovy/json/JsonBuilder.class".to_string(),
            short_name: "JsonBuilder".to_string(),
            fully_qualified_name: "groovy.json.JsonBuilder".to_string(),
            package_name: "groovy.json".to_string(),
            parent_name: Some("groovy.json".to_string()),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["public".to_string()]),
            line_start: 0,
            line_end: 0,
            char_start: 0,
            char_end: 0,
            ident_line_start: 0,
            ident_line_end: 0,
            ident_char_start: 0,
            ident_char_end: 0,
            needs_decompilation: true,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![]),
            },),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn test_index_jdk_dep_source_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let gradle_handler = GradleHandler;
    let dep_jar = gradle_handler
        .get_jdk_dependency_path(&path)
        .expect("Failed to get JDK dependency path");

    println!("Result: {:?}", dep_jar);

    assert!(
        dep_jar.is_some(),
        "JDK dependency source jar should be found"
    );

    let vcs = get_vcs_handler(&path);
    let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_jar(&dep_jar.unwrap())
        .await
        .expect("JAR indexing failed");

    let result = repo
        .find_external_symbol_by_fqn("java.lang.String")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();
    symbol.metadata.documentation = None;

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "java.base/java/lang/String.java".to_string(),
            short_name: "String".to_string(),
            fully_qualified_name: "java.lang.String".to_string(),
            package_name: "java.lang".to_string(),
            parent_name: Some("java.lang".to_string()),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["public".to_string(), "final".to_string()]),
            line_start: 65,
            line_end: 4916,
            char_start: 0,
            char_end: 1,
            ident_line_start: 141,
            ident_line_end: 141,
            ident_char_start: 19,
            ident_char_end: 25,
            needs_decompilation: false,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![]),
            },),
            last_modified: 0,
        }
    );
}
