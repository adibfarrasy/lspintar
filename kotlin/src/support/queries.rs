use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static KOTLIN_TS_LANGUAGE: LazyLock<Language> = LazyLock::new(|| tree_sitter_kotlin::language());

pub static GET_IMPORTS_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&KOTLIN_TS_LANGUAGE, r#"(import_header) @import"#).unwrap());

pub static GET_PACKAGE_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(package_header (qualified_identifier) @package)"#,
    )
    .unwrap()
});

pub static GET_EXTENDS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        (delegation_specifier 
          (constructor_invocation 
            (user_type (type_identifier) @superclass)))
        "#,
    )
    .unwrap()
});

pub static GET_IMPLEMENTS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(delegation_specifier (user_type (type_identifier) @super_interfaces))"#,
    )
    .unwrap()
});

pub static GET_MODIFIERS_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&KOTLIN_TS_LANGUAGE, r#"(modifiers) @modifier"#).unwrap());

pub static GET_FIELD_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(variable_declaration type: (_) @ret)"#,
    )
    .unwrap()
});

pub static GET_FUNCTION_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(function_declaration return_type: (_) @ret)"#,
    )
    .unwrap()
});

pub static DECLARES_VARIABLE_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
            (variable_declaration name: (identifier) @name)
            (parameter name: (identifier) @name)
            (class_parameter name: (identifier) @name)
            (property_declaration (variable_declaration name: (identifier) @name))
        ]
        "#,
    )
    .unwrap()
});

pub static GET_FIELD_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        (property_declaration (variable_declaration name: (identifier) @name))
        "#,
    )
    .unwrap()
});

pub static GET_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
        (class_declaration name: (type_identifier) @name)
        (interface_declaration name: (type_identifier) @name)
        (function_declaration name: (identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

pub static GET_ANNOTATIONS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
          (class_declaration (modifiers (annotation) @annotation))
          (interface_declaration (modifiers (annotation) @annotation))
          (property_declaration (modifiers (annotation) @annotation))
          (function_declaration (modifiers (annotation) @annotation))
        ]
        "#,
    )
    .unwrap()
});

pub static GET_KOTLINDOC_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&KOTLIN_TS_LANGUAGE, r#"(kotlindoc_comment) @doc"#).unwrap());

pub static GET_PARAMETERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(function_declaration (parameters (parameter) @arg))"#,
    )
    .unwrap()
});

pub static IDENT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        (statements (identifier) @trivial_case)
        (navigation_expression
            (_) @nav_qualifier
            (navigation_suffix (identifier) @nav_name))
        (call_expression
            (call_suffix
                (value_arguments
                    (value_argument) @arg_name)))
        (property_declaration value: (identifier) @var_decl)
        (navigation_expression
            (this_expression) @this_qualifier
            (navigation_suffix (identifier) @this_method_name))
        (call_expression
            (identifier) @constructor_type
            (call_suffix))
        (type_projection (user_type (type_identifier) @type_arg))
        (as_expression (user_type (type_identifier) @cast_type))
        (import_header (identifier) @full_import)
        (class_declaration name: (type_identifier) @class_name)
        (interface_declaration name: (type_identifier) @interface_name)
        (function_declaration name: (identifier) @function_name)
        (property_declaration (variable_declaration name: (identifier) @property_name))
        (delegation_specifier (user_type (type_identifier) @super_interfaces))
        (delegation_specifier 
          (constructor_invocation 
            (user_type (type_identifier) @superclass)))
        "#,
    )
    .unwrap()
});
