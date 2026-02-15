use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static JAVA_TS_LANGUAGE: LazyLock<Language> = LazyLock::new(|| tree_sitter_java::LANGUAGE.into());

pub static GET_IMPORTS_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&JAVA_TS_LANGUAGE, r#"(import_declaration) @import"#).unwrap());

pub static GET_PACKAGE_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"(package_declaration (scoped_identifier) @package)"#,
    )
    .unwrap()
});

pub static GET_EXTENDS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"(superclass (type_identifier) @superclass)"#,
    )
    .unwrap()
});

pub static GET_IMPLEMENTS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"(super_interfaces (type_list (type_identifier) @interface))"#,
    )
    .unwrap()
});

pub static GET_MODIFIERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(&JAVA_TS_LANGUAGE, r#"(modifiers ["public" "private" "protected" "static" "final" "abstract" "synchronized" "native" "strictfp" "transient" "volatile"] @modifier)"#).unwrap()
});

pub static GET_FIELD_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"
        (field_declaration type: (_) @ret)
        (constant_declaration type: (_) @ret)
        "#,
    )
    .unwrap()
});

pub static GET_FUNCTION_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"(function_declaration (type_identifier) @ret)"#,
    )
    .unwrap()
});

pub static DECLARES_VARIABLE_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
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
        &JAVA_TS_LANGUAGE,
        r#"
        (field_declaration (variable_declarator name: (identifier) @name))
        (constant_declaration (variable_declarator name: (identifier) @name))
        "#,
    )
    .unwrap()
});

pub static GET_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"
        [
        (class_declaration name: (identifier) @name)
        (interface_declaration name: (identifier) @name)
        (enum_declaration name: (identifier) @name)
        (function_declaration name: (identifier) @name)
        (annotation_type_declaration name: (identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

pub static GET_ANNOTATIONS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
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

pub static GET_JAVADOC_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&JAVA_TS_LANGUAGE, r#"(javadoc_comment) @doc"#).unwrap());

pub static GET_PARAMETERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"(function_declaration (parameters (parameter) @arg))"#,
    )
    .unwrap()
});

pub static IDENT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &JAVA_TS_LANGUAGE,
        r#"
            (expression_statement (identifier) @trivial_case)
            (method_invocation
                object: (_) @method_qualifier
                name: (identifier) @method_name)
            (method_invocation
                object: (this) @this_qualifier
                name: (identifier) @this_method_name)
            (field_access
                object: (_) @field_qualifier
                field: (identifier) @field_name)
            (argument_list (identifier) @arg_name)
            (variable_declarator (identifier) @var_decl)
            [
                (object_creation_expression
                    type: (type_identifier) @constructor_type)
                (object_creation_expression
                    type: (generic_type (type_identifier) @constructor_type))
                (object_creation_expression
                    type: (scoped_type_identifier
                        (_) @scoped_constructor_qualifier
                        (type_identifier) @scoped_constructor_type))
                (object_creation_expression
                    type: (generic_type
                        (scoped_type_identifier
                            (_) @scoped_constructor_qualifier
                            (type_identifier) @scoped_constructor_type)))
            ]
            (type_arguments (type_identifier) @type_arg)
            (cast_expression type: (type_identifier) @cast_type)
            (import_declaration
                (scoped_identifier
                    name: (identifier) @import_name) @full_import)
            (class_declaration name: (identifier) @class_name)
            (interface_declaration name: (identifier) @interface_name)
            (function_declaration name: (identifier) @function_name)
            (field_declaration (variable_declarator name: (identifier) @field_decl_name))
            (super_interfaces (type_list (type_identifier) @super_interfaces))
            (superclass (type_identifier) @superclass)
            (function_declaration type: (type_identifier) @return_name)
        "#,
    )
    .unwrap()
});
