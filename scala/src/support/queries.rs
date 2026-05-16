use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static SCALA_TS_LANGUAGE: LazyLock<Language> =
    LazyLock::new(|| tree_sitter_scala::LANGUAGE.into());

pub static GET_PACKAGE_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &SCALA_TS_LANGUAGE,
        r#"(package_clause (package_identifier) @package)"#,
    )
    .unwrap()
});

pub static GET_IMPORTS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &SCALA_TS_LANGUAGE,
        r#"(import_declaration) @import"#,
    )
    .unwrap()
});

pub static GET_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &SCALA_TS_LANGUAGE,
        r#"
        [
          (class_definition name: (identifier) @name)
          (object_definition name: (identifier) @name)
          (trait_definition name: (identifier) @name)
          (enum_definition name: (identifier) @name)
          (function_definition name: (identifier) @name)
          (function_declaration name: (identifier) @name)
          (type_definition name: (type_identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

pub static GET_VAL_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &SCALA_TS_LANGUAGE,
        r#"
        [
          (val_definition pattern: (identifier) @name)
          (var_definition pattern: (identifier) @name)
          (val_declaration name: (identifier) @name)
          (var_declaration name: (identifier) @name)
          (given_definition name: (identifier) @name)
        ]
        "#,
    )
    .unwrap()
});
