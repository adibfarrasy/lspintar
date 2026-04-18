use std::sync::LazyLock;
use tree_sitter::{Language, Query};

static KOTLIN_TS_LANGUAGE: LazyLock<Language> = LazyLock::new(tree_sitter_kotlin::language);

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

pub static GET_MODIFIERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [

            (modifiers 
            [
                (class_modifier)
                (member_modifier)
                (visibility_modifier)
                (function_modifier)
                (property_modifier)
                (inheritance_modifier)
                (parameter_modifier)
                (platform_modifier)
            ] @modifier
            )
            (binding_pattern_kind) @modifier
        ]
        "#,
    )
    .unwrap()
});

pub static GET_FIELD_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
            (variable_declaration type: (_) @ret)
            (class_parameter type: (user_type (type_identifier) @ret))
        "#,
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
        (class_parameter name: (identifier) @name)
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
            (modifiers [
              (annotation (user_type (type_identifier) @annotation))
              (annotation (constructor_invocation (user_type (type_identifier) @annotation)))
            ])
            (parameter_modifiers [
              (annotation (user_type (type_identifier) @annotation))
              (annotation (constructor_invocation (user_type (type_identifier) @annotation)))
            ])
        ]
        "#,
    )
    .unwrap()
});

pub static GET_KDOC_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&KOTLIN_TS_LANGUAGE, r#"(kdoc_comment) @doc"#).unwrap());

pub static GET_PARAMETERS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"[
            (function_declaration (parameters (parameter) @arg))
            (class_declaration (primary_constructor (class_parameter) @arg))
        ]"#,
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
        (function_declaration return_type: (user_type (type_identifier) @return_name))
        [
            (modifiers [
              (annotation (user_type (type_identifier) @annotation))
              (annotation (constructor_invocation (user_type (type_identifier) @annotation)))
            ])
            (parameter_modifiers [
              (annotation (user_type (type_identifier) @annotation))
              (annotation (constructor_invocation (user_type (type_identifier) @annotation)))
            ])
        ]
        "#,
    )
    .unwrap()
});

/// Captures the names of all type declarations in the file (class, interface, object).
pub static DECLARED_TYPES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
          (class_declaration name: (type_identifier) @name)
          (interface_declaration name: (type_identifier) @name)
          (object_declaration name: (type_identifier) @name)
        ]
        "#,
    )
    .unwrap()
});

/// Captures type identifier usage sites (not declarations).
pub static GET_TYPE_REFS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
          (variable_declaration type: (user_type (type_identifier) @ref))
          (variable_declaration type: (nullable_type (user_type (type_identifier) @ref)))
          (parameter type: (user_type (type_identifier) @ref))
          (parameter type: (nullable_type (user_type (type_identifier) @ref)))
          (class_parameter type: (user_type (type_identifier) @ref))
          (class_parameter type: (nullable_type (user_type (type_identifier) @ref)))
          (type_projection (user_type (type_identifier) @ref))
          (as_expression (user_type (type_identifier) @ref))
          (delegation_specifier (user_type (type_identifier) @ref))
          (delegation_specifier (constructor_invocation (user_type (type_identifier) @ref)))
          (function_declaration return_type: (user_type (type_identifier) @ref))
        ]
        "#,
    )
    .unwrap()
});

/// Captures function declarations with an explicit return type and name.
/// Used to detect missing return statements in non-Unit functions.
pub static FUNCTION_WITH_RETURN_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(function_declaration
          name: (identifier) @name
          return_type: (_) @ret_type)"#,
    )
    .unwrap()
});

/// Captures method names directly defined in a class body.
pub static CLASS_METHOD_NAMES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(function_declaration name: (identifier) @method_name)"#,
    )
    .unwrap()
});

/// Captures qualified member-access call sites `receiver.method(...)`.
pub static GET_MEMBER_ACCESSES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(call_expression
          (navigation_expression
            (identifier) @receiver
            (navigation_suffix (identifier) @method))
          (call_suffix))"#,
    )
    .unwrap()
});

/// Captures parameterised type usages for wrong_type_argument_count detection.
pub static GET_GENERIC_TYPE_USAGES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(user_type (type_identifier) @base (type_arguments) @args)"#,
    )
    .unwrap()
});

/// Captures `override`-modified functions: modifier text and function name.
/// Return type (if any) is extracted from the function_declaration node in code,
/// since it is an optional field not always present.
pub static GET_OVERRIDE_METHODS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(function_declaration
          (modifiers (member_modifier) @mod)
          name: (identifier) @name)"#,
    )
    .unwrap()
});

/// Captures method call sites where the receiver is a simple identifier.
pub static GET_METHOD_CALL_SITES_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"(call_expression
          (navigation_expression
            (identifier) @receiver
            (navigation_suffix (identifier) @method))
          (call_suffix (value_arguments) @args))"#,
    )
    .unwrap()
});

pub static GET_TYPE_QUERY: LazyLock<Query> = LazyLock::new(|| {
    Query::new(
        &KOTLIN_TS_LANGUAGE,
        r#"
        [
          (variable_declaration type: (user_type (type_identifier) @identifier))
          (variable_declaration type: (nullable_type (user_type (type_identifier) @identifier)))
          (parameter type: (user_type (type_identifier) @identifier))
          (parameter type: (nullable_type (user_type (type_identifier) @identifier)))
          (class_parameter type: (user_type (type_identifier) @identifier))
          (class_parameter type: (nullable_type (user_type (type_identifier) @identifier)))
          (type_projection (user_type (type_identifier) @identifier))
          (function_declaration return_type: (user_type (type_identifier) @identifier))
          (interface_declaration name: (type_identifier) @identifier)
          (class_declaration name: (type_identifier) @identifier)
        ]
        "#,
    )
    .unwrap()
});
