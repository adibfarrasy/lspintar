use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static GROOVY_TS_LANGUAGE: LazyLock<Language> = LazyLock::new(|| tree_sitter_groovy::language());

pub static GET_IMPORTS_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&GROOVY_TS_LANGUAGE, r#"(import_declaration) @doc"#).unwrap());

pub static GET_PACKAGE_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(package_declaration (scoped_identifier) @package)"#,
    )
    .unwrap()
});

pub static GET_EXTENDS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(superclass (type_identifier) @superclass)"#,
    )
    .unwrap()
});

pub static GET_IMPLEMENTS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(super_interfaces (type_list (type_identifier) @interface))"#,
    )
    .unwrap()
});

pub static GET_MODIFIERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(modifiers ["public" "private" "protected" "static" "final" "abstract" "synchronized" "native" "strictfp" "transient" "volatile"] @modifier)"#
    ).unwrap()
});

pub static GET_FIELD_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        (field_declaration type: (_) @ret)
        (constant_declaration type: (_) @ret)
        "#,
    )
    .unwrap()
});

pub static GET_FUNCTION_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(function_declaration (type_identifier) @ret)"#,
    )
    .unwrap()
});

pub static DECLARES_VARIABLE_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
            (variable_declarator name: (identifier) @name)
            (parameter name: (identifier) @name)
            (field_declaration (variable_declarator name: (identifier) @name))
        ]
        "#,
    )
    .unwrap()
});

pub static GET_FIELD_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        (field_declaration (variable_declarator name: (identifier) @name))
        (constant_declaration (variable_declarator name: (identifier) @name))
        "#,
    )
    .unwrap()
});

pub static GET_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
        (class_declaration name: (identifier) @name)
        (interface_declaration name: (identifier) @name)
        (enum_declaration name: (identifier) @name)
        (function_declaration name: (identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

pub static GET_ANNOTATIONS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
          (class_declaration (modifiers [(marker_annotation name: (identifier) @annotation) (annotation name: (identifier) @annotation)]))
          (interface_declaration (modifiers [(marker_annotation name: (identifier) @annotation) (annotation name: (identifier) @annotation)]))
          (enum_declaration (modifiers [(marker_annotation name: (identifier) @annotation) (annotation name: (identifier) @annotation)]))
          (field_declaration (modifiers [(marker_annotation name: (identifier) @annotation) (annotation name: (identifier) @annotation)]))
          (function_declaration (modifiers [(marker_annotation name: (identifier) @annotation) (annotation name: (identifier) @annotation)]))
        ]
        "#
    ).unwrap()
});

pub static GET_GROOVYDOC_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&GROOVY_TS_LANGUAGE, r#"(groovydoc_comment) @doc"#).unwrap());

pub static GET_PARAMETERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(function_declaration (parameters (parameter) @arg))"#,
    )
    .unwrap()
});
