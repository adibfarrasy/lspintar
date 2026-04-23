use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static GROOVY_TS_LANGUAGE: LazyLock<Language> = LazyLock::new(tree_sitter_groovy::language);

pub static GET_IMPORTS_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&GROOVY_TS_LANGUAGE, r#"(import_declaration) @import"#).unwrap());

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

pub static GET_SHORT_NAME_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
        (class_declaration name: (identifier) @name)
        (interface_declaration name: (identifier) @name)
        (enum_declaration name: (identifier) @name)
        (function_declaration name: [(identifier) (string_literal)] @name)
        (annotation_type_declaration name: (identifier) @name)
        (field_declaration (variable_declarator name: (identifier) @name))
        (constant_declaration (variable_declarator name: (identifier) @name))
        ]
        "#,
    )
    .unwrap()
});

pub static GET_ANNOTATIONS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        (modifiers [(marker_annotation name: (identifier) @annotation)
            (annotation name: (identifier) @annotation)])
        "#,
    )
    .unwrap()
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

pub static IDENT_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
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
    (modifiers [(marker_annotation name: (identifier) @annotation)
        (annotation name: (identifier) @annotation)])
"#,
    )
    .unwrap()
});

/// Captures the names of all type declarations in the file (class, interface, enum, annotation).
pub static DECLARED_TYPES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
          (class_declaration name: (identifier) @name)
          (interface_declaration name: (identifier) @name)
          (enum_declaration name: (identifier) @name)
          (annotation_type_declaration name: (identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

/// Captures type identifier usage sites (not declarations).
pub static GET_TYPE_REFS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
          (field_declaration type: (type_identifier) @ref)
          (variable_declaration type: (type_identifier) @ref)
          (parameter type: (type_identifier) @ref)
          (array_type (type_identifier) @ref)
          (generic_type (type_identifier) @ref)
          (generic_type (type_arguments (type_identifier) @ref))
          (object_creation_expression type: (type_identifier) @ref)
          (cast_expression type: (type_identifier) @ref)
          (superclass (type_identifier) @ref)
          (super_interfaces (type_list (type_identifier) @ref))
          (function_declaration type: (type_identifier) @ref)
        ]
        "#,
    )
    .unwrap()
});

/// Captures `new T(...)` expressions.
pub static GET_OBJECT_CREATIONS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"[
          (object_creation_expression type: (type_identifier) @type_name)
          (object_creation_expression type: (generic_type (type_identifier) @type_name))
        ]"#,
    )
    .unwrap()
});

/// Captures qualified member-access call sites `receiver.method(...)`.
pub static GET_MEMBER_ACCESSES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(method_invocation
          object: (identifier) @receiver
          name: (identifier) @method)"#,
    )
    .unwrap()
});

/// Captures parameterised type usages for wrong_type_argument_count detection.
pub static GET_GENERIC_TYPE_USAGES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(generic_type (type_identifier) @base (type_arguments) @args)"#,
    )
    .unwrap()
});

/// Captures @Override-annotated methods: annotation name, method name, return type.
pub static GET_OVERRIDE_METHODS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(function_declaration
          (modifiers (marker_annotation name: (identifier) @ann))
          type: (_) @ret
          name: (identifier) @name)"#,
    )
    .unwrap()
});

/// Captures local variable declarations where a numeric primitive is assigned from an identifier.
pub static GET_NARROWING_CANDIDATES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(variable_declaration
          type: (_) @decl_type
          declarator: (variable_declarator
            name: (identifier) @decl_name
            value: (identifier) @rhs_name))"#,
    )
    .unwrap()
});

/// Captures method call sites where the receiver is a simple identifier.
pub static GET_METHOD_CALL_SITES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"(method_invocation
          object: (identifier) @receiver
          name: (identifier) @method
          arguments: (argument_list) @args)"#,
    )
    .unwrap()
});

pub static GET_TYPE_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &GROOVY_TS_LANGUAGE,
        r#"
        [
          (field_declaration type: (type_identifier) @identifier)
          (variable_declaration type: (type_identifier) @identifier)
          (parameter type: (type_identifier) @identifier)
          (interface_declaration name: (identifier) @identifier)
          (class_declaration name: (identifier) @identifier)
          (enum_declaration name: (identifier) @identifier)
          (array_type (type_identifier) @identifier)
          (class_literal (type_identifier) @identifier)
          (generic_type (type_identifier) @identifier)
          (generic_type (type_arguments (type_identifier) @identifier))
        ]
        "#,
    )
    .unwrap()
});
