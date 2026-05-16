use groovy::GroovySupport;
use java::JavaSupport;
use kotlin::KotlinSupport;
use lsp_core::build_tools::{BuildToolHandler, gradle::GradleHandler};
use lspintar_server::{
    Indexer, Repository,
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata, SymbolParameter},
    },
};
use pretty_assertions::assert_eq;
use sqlx::types::Json;
use std::{path::Path, sync::Arc};
use uuid::Uuid;

#[tokio::test]
async fn index_groovy_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path =
        Path::new("tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

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
            short_name: "User".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec!["CompileStatic".to_string()]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_groovy_gradle_single_workspace() {
    let repo = Arc::new(Repository::new(":memory:").await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
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
            short_name: "UserService".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.UserService".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
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
            short_name: "Repository".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.Repository".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/Repository.groovy"
                    .to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.User#getDisplayName")
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
            short_name: "getDisplayName".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User#getDisplayName".to_string(),
            parent_name: Some("com.example.User".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/User.groovy"
                    .to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.UserService#userVariable")
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
            short_name: "userVariable".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.UserService#userVariable".to_string(),
            parent_name: Some("com.example.UserService".to_string()),
            file_path:
                "tests/fixtures/groovy-gradle-single/src/main/groovy/com/example/UserService.groovy"
                    .to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

// --- Extras.groovy: new fixture exercising nested generic class, enum,
//     static method, and static final field ---------------------------

#[tokio::test]
async fn index_groovy_static_final_field() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let sym = repo
        .find_symbol_by_fqn("com.example.Extras#MAX_RETRIES")
        .await
        .expect("Query failed")
        .expect("MAX_RETRIES should be indexed");

    assert_eq!(sym.short_name, "MAX_RETRIES");
    assert_eq!(sym.symbol_type, "Field");
    // Pin modifiers — `static final` declaration should yield both modifiers.
    let mods: &Vec<String> = &sym.modifiers.0;
    assert!(mods.iter().any(|m| m == "static"), "expected static modifier, got: {mods:?}");
    assert!(mods.iter().any(|m| m == "final"), "expected final modifier, got: {mods:?}");
    assert_eq!(
        sym.metadata.0.return_type.as_deref(),
        Some("int"),
        "expected return_type int for MAX_RETRIES"
    );
}

#[tokio::test]
async fn index_groovy_static_method() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let sym = repo
        .find_symbol_by_fqn("com.example.Extras#describe")
        .await
        .expect("Query failed")
        .expect("Extras.describe should be indexed");

    assert_eq!(sym.short_name, "describe");
    assert_eq!(sym.symbol_type, "Function");
    let mods: &Vec<String> = &sym.modifiers.0;
    assert!(mods.iter().any(|m| m == "static"), "expected static modifier, got: {mods:?}");
}

#[tokio::test]
async fn index_groovy_generic_nested_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let sym = repo
        .find_symbol_by_fqn("com.example.Extras#Pair")
        .await
        .expect("Query failed")
        .expect("nested Pair<A, B> class should be indexed");

    assert_eq!(sym.short_name, "Pair");
    assert_eq!(sym.symbol_type, "Class");
    assert_eq!(
        sym.parent_name.as_deref(),
        Some("com.example.Extras"),
        "nested class parent should be the enclosing class"
    );
}

#[tokio::test]
async fn index_groovy_enum_nested_in_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let sym = repo
        .find_symbol_by_fqn("com.example.Extras#Severity")
        .await
        .expect("Query failed")
        .expect("nested Severity enum should be indexed");

    assert_eq!(sym.short_name, "Severity");
    // Pin symbol_type — `Enum` or `Class` depending on how groovy support
    // models enums. Whatever it is, the lookup must succeed.
    assert!(
        sym.symbol_type == "Enum" || sym.symbol_type == "Class",
        "unexpected symbol_type for nested enum: {}",
        sym.symbol_type
    );
}

#[tokio::test]
async fn index_groovy_class_multi_project() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
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
            short_name: "BaseService".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.BaseService".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_groovy_method() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.api.UserController#execute")
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
            short_name: "execute".to_string(),
            package_name: "com.example.api".to_string(),
            fully_qualified_name: "com.example.api.UserController#execute".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec!["Override".to_string()]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_groovy_nested_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.api.UserController#ApiResponse")
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
            short_name: "ApiResponse".to_string(),
            package_name: "com.example.api".to_string(),
            fully_qualified_name: "com.example.api.UserController#ApiResponse".to_string(),
            parent_name: Some("com.example.api.UserController".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/api/src/main/groovy/com/example/api/UserController.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_groovy_field() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let result = repo
        .find_symbol_by_fqn("com.example.core.DataProcessor#MAX_BATCH_SIZE")
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
            short_name: "MAX_BATCH_SIZE".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.DataProcessor#MAX_BATCH_SIZE".to_string(),
            parent_name: Some("com.example.core.DataProcessor".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_groovy_inheritance() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-multi");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    let results = repo
        .find_supers_by_symbol_fqn("com.example.api.UserController")
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
            short_name: "BaseService".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.BaseService".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/BaseService.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );

    let super_interface = &results[1];
    assert_eq!(
        super_interface,
        &Symbol {
            id: None,
            short_name: "DataProcessor".to_string(),
            package_name: "com.example.core".to_string(),
            fully_qualified_name: "com.example.core.DataProcessor".to_string(),
            parent_name: Some("com.example.core".to_string()),
            file_path: "tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy".to_string(),
            file_type: "groovy".to_string(),
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
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

// Regression: a Kotlin file containing a property without an initializer
// (e.g. `lateinit var`) used to make `get_ident_range` return None on the
// property, which the indexer treats as a hard error and silently drops
// every symbol from the file. The class itself then became invisible to
// `find_all_source_file_paths`, breaking find-references and rename for any
// type it consumed.
#[tokio::test]
async fn index_kotlin_class_with_lateinit_var_property() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/polyglot-spring");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("kt", Arc::new(KotlinSupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

    // The class itself must be indexed.
    let class_sym = repo
        .find_symbol_by_fqn("com.example.KotlinConsumer")
        .await
        .expect("Query failed")
        .expect("KotlinConsumer class must be indexed");
    assert_eq!(class_sym.symbol_type, "Class");

    // The `lateinit var groovyService: GroovyService` property must be indexed too.
    let prop_sym = repo
        .find_symbol_by_fqn("com.example.KotlinConsumer#groovyService")
        .await
        .expect("Query failed")
        .expect("lateinit var groovyService must be indexed");
    assert_eq!(prop_sym.symbol_type, "Field");
}

#[tokio::test]
async fn index_kotlin_data_class() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("kt", Arc::new(KotlinSupport::new()));
    indexer
        .index_workspace(&path, |_, _| {}, |_, _| {})
        .await
        .expect("Indexing failed");

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
            short_name: "User".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User".to_string(),
            parent_name: Some("com.example".to_string()),
            file_path: "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt"
                .to_string(),
            file_type: "kotlin".to_string(),
            symbol_type: "Class".to_string(),
            modifiers: Json(vec!["data".to_string()]),
            line_start: 2,
            line_end: 7,
            char_start: 0,
            char_end: 1,
            ident_line_start: 2,
            ident_line_end: 2,
            ident_char_start: 11,
            ident_char_end: 15,
            metadata: Json(SymbolMetadata {
                parameters: Some(vec![
                    SymbolParameter {
                        name: "val id".to_string(),
                        type_name: Some("Long".to_string()),
                        default_value: None,
                    },
                    SymbolParameter {
                        name: "val name".to_string(),
                        type_name: Some("String".to_string()),
                        default_value: None,
                    },
                    SymbolParameter {
                        name: "val status".to_string(),
                        type_name: Some("String".to_string()),
                        default_value: None,
                    },
                    SymbolParameter {
                        name: "val occupation".to_string(),
                        type_name: Some("String".to_string()),
                        default_value: Some("\"unemployed\"".to_string()),
                    },
                ],),
                return_type: None,
                documentation: None,
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );

    let result = repo
        .find_symbol_by_fqn("com.example.User#name")
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
            short_name: "name".to_string(),
            package_name: "com.example".to_string(),
            fully_qualified_name: "com.example.User#name".to_string(),
            parent_name: Some("com.example.User".to_string()),
            file_path: "tests/fixtures/polyglot-spring/src/main/kotlin/com/example/demo/User.kt"
                .to_string(),
            file_type: "kotlin".to_string(),
            symbol_type: "Field".to_string(),
            modifiers: Json(vec!["val".to_string()]),
            line_start: 4,
            line_end: 4,
            char_start: 4,
            char_end: 20,
            ident_line_start: 4,
            ident_line_end: 4,
            ident_char_start: 8,
            ident_char_end: 12,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: Some("String".to_string()),
                documentation: None,
                annotations: Some(vec![]),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            }),
            last_modified: 0,
        }
    );
}

#[tokio::test]
async fn index_external_dep_source_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
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

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_external_deps(
            vec![(Some(jar_path.clone()), Some(jar_path))],
            |_, _| {},
            |_, _| {},
        )
        .await;

    let result = repo
        .find_external_symbol_by_fqn("groovy.json.JsonBuilder")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();
    symbol.alt_jar_path = None;

    let doc_string = "/**\n * A builder for creating JSON payloads.\n * <p>\n * This builder supports the usual builder syntax made of nested method calls and closures,\n * but also some specific aspects of JSON data structures, such as list of values, etc.\n * Please make sure to have a look at the various methods provided by this builder\n * to be able to learn about the various possibilities of usage.\n * <p>\n * Example:\n * <pre><code class=\"groovyTestCase\">\n *       def builder = new groovy.json.JsonBuilder()\n *       def root = builder.people {\n *           person {\n *               firstName 'Guillaume'\n *               lastName 'Laforge'\n *               // Named arguments are valid values for objects too\n *               address(\n *                       city: 'Paris',\n *                       country: 'France',\n *                       zip: 12345,\n *               )\n *               married true\n *               // a list of values\n *               conferences 'JavaOne', 'Gr8conf'\n *           }\n *       }\n *\n *       // creates a data structure made of maps (Json object) and lists (Json array)\n *       assert root instanceof Map\n *\n *       assert builder.toString() == '{\"people\":{\"person\":{\"firstName\":\"Guillaume\",\"lastName\":\"Laforge\",\"address\":{\"city\":\"Paris\",\"country\":\"France\",\"zip\":12345},\"married\":true,\"conferences\":[\"JavaOne\",\"Gr8conf\"]}}}'\n * </code></pre>\n *\n * @since 1.8.0\n */";

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "groovy/json/JsonBuilder.java".to_string(),
            alt_jar_path: None,
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
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            },),
            last_modified: 0,
            file_type: "java".to_string(),
        }
    );
}

#[tokio::test]
async fn index_external_dep_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let gradle_handler = GradleHandler;
    let dep_jars = gradle_handler.get_dependency_paths(&path).unwrap();

    let jar_path = dep_jars
        .iter()
        .map(|p| p.clone().0)
        .find(|jar| {
            jar.as_ref()
                .expect("groovy-json bytecode class jar not found")
                .to_string_lossy()
                .contains("groovy-json")
                && !jar
                    .as_ref()
                    .expect("groovy-json bytecode class jar not found")
                    .to_string_lossy()
                    .contains("-sources.jar")
        })
        .expect("groovy-json bytecode class jar not found");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_external_deps(vec![(jar_path.clone(), jar_path)], |_, _| {}, |_, _| {})
        .await;

    let result = repo
        .find_external_symbol_by_fqn("groovy.json.JsonBuilder")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();
    symbol.alt_jar_path = None;

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "groovy/json/JsonBuilder.class".to_string(),
            alt_jar_path: None,
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
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            },),
            last_modified: 0,
            file_type: "java".to_string(),
        }
    );
}

#[tokio::test]
async fn index_jdk_dep_source_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/groovy-gradle-single");

    let gradle_handler = GradleHandler;
    let dep_jar = gradle_handler
        .get_jdk_dependency_path(&path)
        .expect("Failed to get JDK dependency path");

    assert!(
        dep_jar.is_some(),
        "JDK dependency source jar should be found"
    );

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_external_deps(vec![(None, dep_jar)], |_, _| {}, |_, _| {})
        .await;

    let result = repo
        .find_external_symbol_by_fqn("java.lang.String")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let symbol = result.unwrap();

    // JDK-stable assertions — these must hold for any JDK version that ships
    // String.java in src.zip.
    assert_eq!(symbol.short_name, "String");
    assert_eq!(symbol.fully_qualified_name, "java.lang.String");
    assert_eq!(symbol.package_name, "java.lang");
    assert_eq!(symbol.parent_name.as_deref(), Some("java.lang"));
    assert_eq!(symbol.symbol_type, "Class");
    assert_eq!(symbol.file_type, "java");
    assert_eq!(symbol.source_file_path, "java.base/java/lang/String.java");
    assert!(
        symbol.modifiers.0.iter().any(|m| m == "public"),
        "String should be public, got: {:?}",
        symbol.modifiers.0
    );
    assert!(
        symbol.modifiers.0.iter().any(|m| m == "final"),
        "String should be final, got: {:?}",
        symbol.modifiers.0
    );
    assert!(!symbol.needs_decompilation, "source jar dep is not decompiled");
    assert!(symbol.metadata.0.annotations.as_ref().map(|a| a.is_empty()).unwrap_or(true));

    // JDK-version-dependent — line counts and identifier columns shift between
    // releases. Just check they're plausible rather than pinning exact numbers.
    assert!(
        symbol.line_start > 0 && symbol.line_end > symbol.line_start,
        "implausible line range: {}..{}",
        symbol.line_start,
        symbol.line_end
    );
    assert!(
        symbol.line_end - symbol.line_start > 1000,
        "String.java should span thousands of lines, got: {}..{}",
        symbol.line_start,
        symbol.line_end
    );
    assert!(
        symbol.ident_line_start >= symbol.line_start
            && symbol.ident_line_end <= symbol.line_end,
        "identifier range {}..{} must sit inside class range {}..{}",
        symbol.ident_line_start,
        symbol.ident_line_end,
        symbol.line_start,
        symbol.line_end
    );
}

#[tokio::test]
async fn index_external_annotation_dep_jar() {
    let db_name = Uuid::new_v4();
    let db_dir = format!("file:{}?mode=memory", db_name);
    let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
    let path = Path::new("tests/fixtures/polyglot-spring");

    let gradle_handler = GradleHandler;
    let dep_jars = gradle_handler.get_dependency_paths(&path).unwrap();

    let jar_path = dep_jars
        .iter()
        .map(|p| p.clone().1)
        .find(|p| {
            if let Some(source_jar) = p {
                source_jar.to_string_lossy().contains("spring-context")
                    && source_jar.to_string_lossy().contains("-sources.jar")
            } else {
                false
            }
        })
        .expect("spring-context sources.jar not found")
        .expect("spring-context is empty");

    let mut indexer = Indexer::new(Arc::clone(&repo));
    indexer.register_language("groovy", Arc::new(GroovySupport::new()));
    indexer.register_language("java", Arc::new(JavaSupport::new()));
    indexer
        .index_external_deps(
            vec![(Some(jar_path.clone()), Some(jar_path))],
            |_, _| {},
            |_, _| {},
        )
        .await;

    let result = repo
        .find_external_symbol_by_fqn("org.springframework.stereotype.Service")
        .await
        .expect("Query failed");
    assert!(result.is_some(), "External symbol should be found");

    let mut symbol = result.unwrap();
    symbol.id = None;
    symbol.last_modified = 0;
    symbol.jar_path = String::new();
    symbol.metadata.documentation = None;
    symbol.alt_jar_path = None;

    assert_eq!(
        symbol,
        ExternalSymbol {
            id: None,
            jar_path: String::new(),
            source_file_path: "org/springframework/stereotype/Service.java".to_string(),
            alt_jar_path: None,
            short_name: "Service".to_string(),
            fully_qualified_name: "org.springframework.stereotype.Service".to_string(),
            package_name: "org.springframework.stereotype".to_string(),
            parent_name: Some("org.springframework.stereotype".to_string()),
            symbol_type: "Annotation".to_string(),
            modifiers: Json(vec!["public".to_string()]),
            line_start: 26,
            line_end: 57,
            char_start: 0,
            char_end: 1,
            ident_line_start: 47,
            ident_line_end: 47,
            ident_char_start: 18,
            ident_char_end: 25,
            needs_decompilation: false,
            metadata: Json(SymbolMetadata {
                parameters: None,
                return_type: None,
                documentation: None,
                annotations: Some(vec![
                    "Target".to_string(),
                    "Retention".to_string(),
                    "Documented".to_string(),
                    "Component".to_string()
                ],),
                generic_return_type: None,
                type_params: None,
                generic_param_types: None,
                method_type_params: None,
            },),
            last_modified: 0,
            file_type: "java".to_string(),
        }
    );
}
