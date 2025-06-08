use std::{path::PathBuf, sync::Arc, usize};

use anyhow::{anyhow, Context, Result};
use log::debug;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{
            create_parser_for_language, detect_language_from_path, find_project_root,
            path_to_file_uri, uri_to_path,
        },
    },
    languages::groovy::symbols::SymbolType,
};

pub fn find_local(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
) -> Option<Location> {
    let definition_node = search_local_definitions(tree, source, usage_node)?;

    node_to_lsp_location(&definition_node, file_uri)
}

pub fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
) -> Option<Node<'a>> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_type = determine_symbol_type_from_context(tree, usage_node, source).ok()?;

    let query_text = match symbol_type {
        SymbolType::Function => r#"(method_declaration name: (identifier) @name)"#,
        SymbolType::Class => r#"(class_declaration name: (identifier) @name)"#,
        SymbolType::Interface => r#"(interface_declaration name: (identifier) @name)"#,
        SymbolType::Method => r#"(method_declaration name: (identifier) @name)"#,
        SymbolType::Field => {
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#
        }
        SymbolType::Variable => {
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
            "#
        }
        SymbolType::Parameter => r#"(formal_parameter name: (identifier) @name)"#,
        SymbolType::Enum => r#"(enum_declaration name: (identifier) @name)"#,
        _ => return None,
    };

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut candidates = Vec::new();
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let node = capture.node;
                let node_text = node.utf8_text(source.as_bytes()).unwrap();
                if node_text == symbol_name {
                    candidates.push(node.parent().unwrap());
                };
            }
        });

    match symbol_type {
        SymbolType::Variable | SymbolType::Parameter => {
            find_closest_declaration(usage_node, &candidates)
        }

        SymbolType::Method | SymbolType::Function => {
            find_best_method_match(tree, source, usage_node, symbol_name)
        }
        _ => candidates.into_iter().next(),
    }
}

fn find_closest_declaration<'a>(usage_node: &Node, candidates: &[Node<'a>]) -> Option<Node<'a>> {
    let mut best_candidate = None;
    let mut best_scope_distance = usize::MAX;

    for candidate in candidates {
        if let Some(distance) = calculate_scope_distance(usage_node, candidate) {
            if distance < best_scope_distance {
                best_scope_distance = distance;
                best_candidate = Some(*candidate);
            }
        }
    }

    best_candidate
}

fn calculate_scope_distance(usage_node: &Node, declaration_node: &Node) -> Option<usize> {
    // Check if declaration is in scope of usage
    if !is_in_scope(usage_node, declaration_node) {
        return None;
    }

    // Calculate nesting distance
    let usage_depth = get_nesting_depth(usage_node);
    let decl_depth = get_nesting_depth(declaration_node);

    // Prefer closer scopes (higher depth difference means closer)
    Some(usage_depth.saturating_sub(decl_depth))
}

fn is_in_scope(usage_node: &Node, declaration_node: &Node) -> bool {
    // For formal parameters, check if usage is in the same method
    if let Some(decl_method) = find_containing_method(declaration_node) {
        if let Some(usage_method) = find_containing_method(usage_node) {
            return decl_method.id() == usage_method.id();
        }
    }

    // For local variables, check if declaration comes before usage in same block
    if let Some(decl_block) = find_containing_block(declaration_node) {
        if let Some(usage_block) = find_containing_block(usage_node) {
            if decl_block.id() == usage_block.id() {
                // Check if declaration comes before usage
                return declaration_node.start_position() < usage_node.start_position();
            }
        }
    }

    false
}

fn find_containing_method<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_declaration" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn find_containing_block<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "block" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn get_nesting_depth(node: &Node) -> usize {
    let mut depth = 0;
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "block" | "method_declaration" | "class_declaration"
        ) {
            depth += 1;
        }
        current = parent.parent();
    }
    depth
}

fn node_to_lsp_location(node: &Node, file_uri: &str) -> Option<Location> {
    let start_pos = node.start_position();
    let end_pos = node.end_position();

    let range = Range {
        start: Position {
            line: start_pos.row as u32,
            character: start_pos.column as u32,
        },
        end: Position {
            line: end_pos.row as u32,
            character: end_pos.column as u32,
        },
    };

    let uri = Url::parse(file_uri).ok()?;
    Some(Location { uri, range })
}

#[derive(Debug, Clone)]
pub struct CallSignature {
    pub arg_count: usize,
    pub arg_types: Vec<Option<String>>, // None if type can't be inferred
}

#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub param_count: usize,
    pub param_types: Vec<String>,
    pub param_names: Vec<String>,
}

fn extract_call_signature(usage_node: &Node, source: &str) -> Option<CallSignature> {
    // Navigate up to find method_invocation parent
    let method_invocation = find_parent_method_invocation(usage_node)?;

    // Get the arguments node
    let arguments = method_invocation.child_by_field_name("arguments")?;

    let mut arg_types = Vec::new();
    let mut cursor = arguments.walk();

    // Iterate through argument children
    for child in arguments.named_children(&mut cursor) {
        let arg_type = infer_argument_type(&child, source);
        arg_types.push(arg_type);
    }

    Some(CallSignature {
        arg_count: arg_types.len(),
        arg_types,
    })
}

fn extract_method_signature(method_node: &Node, source: &str) -> Option<MethodSignature> {
    // method_node should be a method_declaration
    if method_node.kind() != "method_declaration" {
        return None;
    }

    // Find parameters
    let parameters = method_node.child_by_field_name("parameters")?;

    let mut param_types = Vec::new();
    let mut param_names = Vec::new();
    let mut cursor = parameters.walk();

    let mut has_spread = false;

    for child in parameters.named_children(&mut cursor) {
        if vec!["formal_parameter", "spread_parameter"].contains(&child.kind()) {
            // Extract type and name from formal_parameter
            if let Some(param_type) = child.child_by_field_name("type") {
                param_types.push(
                    param_type
                        .utf8_text(source.as_bytes())
                        .unwrap_or("")
                        .to_string(),
                );
            } else {
                param_types.push("def".to_string()); // Groovy default
            }

            if let Some(param_name) = child.child_by_field_name("name") {
                param_names.push(
                    param_name
                        .utf8_text(source.as_bytes())
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }

        if child.kind() == "spread_parameter" {
            has_spread = true;
        }
    }

    Some(MethodSignature {
        param_count: if has_spread {
            usize::MAX
        } else {
            param_types.len()
        },
        param_types,
        param_names,
    })
}

fn signatures_match(call_sig: &CallSignature, method_sig: &MethodSignature) -> bool {
    // If we have type information, try to match types
    for (i, call_arg_type) in call_sig.arg_types.iter().enumerate() {
        if let Some(method_param_type) = method_sig.param_types.get(i) {
            if let Some(call_type) = call_arg_type {
                if !types_compatible(call_type, method_param_type) {
                    return false;
                }
            }
            // If call_type is None (can't infer), assume compatible
        }
    }

    if call_sig.arg_count != method_sig.param_count && method_sig.param_count < usize::MAX {
        return false;
    }

    true
}

fn find_parent_method_invocation<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_invocation" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn infer_argument_type(arg_node: &Node, source: &str) -> Option<String> {
    match arg_node.kind() {
        // Integer literals
        "decimal_integer_literal" => Some("int".to_string()),
        "hex_integer_literal" => Some("int".to_string()),
        "octal_integer_literal" => Some("int".to_string()),
        "binary_integer_literal" => Some("int".to_string()),

        // Floating point literals
        "decimal_floating_point_literal" => Some("double".to_string()),
        "hex_floating_point_literal" => Some("double".to_string()),

        // Boolean literals
        "true" | "false" => Some("boolean".to_string()),

        // Character and string literals
        "character_literal" => Some("char".to_string()),
        "string_literal" => Some("String".to_string()),
        "text_block" => Some("String".to_string()), // Multi-line string

        // Null literal
        "null_literal" => Some("null".to_string()),

        // Collection literals
        "map_literal" => Some("Map".to_string()),
        "array_literal" => {
            // In Groovy, [1, 2, 3] creates a List, not an array
            Some("List".to_string())
        }

        // Complex expressions
        "identifier" => {
            // Could enhance this with variable type tracking
            // For now, return None (unknown type)
            None
        }
        "method_invocation" => {
            // Could enhance with return type inference
            None
        }
        "field_access" => {
            // Could enhance with field type lookup
            None
        }
        "cast_expression" => {
            // Extract the target type from the cast
            if let Some(type_node) = arg_node.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(type_text.to_string())
            } else {
                None
            }
        }
        "parenthesized_expression" => {
            // Recurse into the parenthesized expression
            if let Some(inner_expr) = arg_node.child_by_field_name("expression") {
                infer_argument_type(&inner_expr, source)
            } else {
                None
            }
        }

        // Constructor calls
        "object_creation_expression" => {
            if let Some(type_node) = arg_node.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(type_text.to_string())
            } else {
                None
            }
        }

        // Binary operations - try to infer result type
        "binary_expression" => {
            // Basic inference for arithmetic operations
            if let Some(operator) = arg_node.child_by_field_name("operator") {
                let op_text = operator.utf8_text(source.as_bytes()).ok()?;
                match op_text {
                    "+" | "-" | "*" | "/" | "%" => {
                        // Very basic: if any operand is floating point, result is double
                        if contains_floating_point_operand(arg_node, source) {
                            Some("double".to_string())
                        } else {
                            Some("int".to_string())
                        }
                    }
                    "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" => {
                        Some("boolean".to_string())
                    }
                    _ => None,
                }
            } else {
                None
            }
        }

        // Ternary operator
        "ternary_expression" => {
            // Try to infer from the true/false branches
            if let Some(true_expr) = arg_node.child_by_field_name("consequence") {
                infer_argument_type(&true_expr, source)
            } else if let Some(false_expr) = arg_node.child_by_field_name("alternative") {
                infer_argument_type(&false_expr, source)
            } else {
                None
            }
        }

        _ => None,
    }
}

fn contains_floating_point_operand(binary_expr: &Node, source: &str) -> bool {
    let mut cursor = binary_expr.walk();
    for child in binary_expr.children(&mut cursor) {
        match child.kind() {
            "decimal_floating_point_literal" | "hex_floating_point_literal" => return true,
            "identifier" => {
                // Could enhance with variable type lookup
                // For now, conservatively assume it might be floating point
                continue;
            }
            _ => continue,
        }
    }
    false
}

fn types_compatible(call_type: &str, param_type: &str) -> bool {
    match (call_type, param_type) {
        // Exact match
        (a, b) if a == b => true,

        // Groovy's def accepts anything
        (_, "def") => true,
        ("def", _) => true,

        // Object accepts anything (boxing)
        (_, "Object") => true,

        // Numeric conversions (Groovy auto-boxing/widening)
        ("int", "Integer") => true,
        ("Integer", "int") => true,
        ("int", "long") => true,
        ("int", "Long") => true,
        ("int", "double") => true,
        ("int", "Double") => true,
        ("double", "Double") => true,
        ("Double", "double") => true,
        ("boolean", "Boolean") => true,
        ("Boolean", "boolean") => true,
        ("char", "Character") => true,
        ("Character", "char") => true,

        // String conversions (Groovy's GString)
        ("String", "GString") => true,
        ("GString", "String") => true,

        // Collection interface compatibility
        ("List", "Collection") => true,
        ("Map", "Object") => true,
        ("List", "Object") => true,

        // Null compatibility with reference types
        ("null", param_type) if !is_primitive_type(param_type) => true,

        _ => false,
    }
}

fn is_primitive_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "int" | "double" | "boolean" | "char" | "long" | "float" | "byte" | "short"
    )
}

fn find_best_method_match<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Option<Node<'a>> {
    // Extract call signature
    let call_signature = extract_call_signature(usage_node, source)?;

    // Find all method candidates with matching name
    let query_text = r#"(method_declaration name: (identifier) @name)"#;
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut best_match = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let name_node = capture.node;
                let name_text = name_node.utf8_text(source.as_bytes()).unwrap_or("");

                debug!(
                    "name_node: {:#?}, name_text: {}, symbol_name: {}",
                    name_node, name_text, symbol_name
                );

                if name_text == symbol_name {
                    // Found a method with matching name
                    if let Some(method_decl) = name_node.parent() {
                        if let Some(method_sig) = extract_method_signature(&method_decl, source) {
                            if signatures_match(&call_signature, &method_sig) {
                                best_match = Some(method_decl);
                                return; // Take first match for now
                            }
                        }
                    }
                }
            }
        });

    best_match
}

fn determine_symbol_type_from_context(
    tree: &Tree,
    node: &Node,
    source: &str,
) -> Result<SymbolType> {
    let node_text = node.utf8_text(source.as_bytes())?;

    if let Some(manual_type) = check_complex_structures(node) {
        return Ok(manual_type);
    }

    let query_text = r#"
        ; DECLARATIONS
        ; Variable declarations
        (variable_declaration
          declarator: (variable_declarator
            name: (identifier) @variable_name))

        ; Field declarations  
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_name))

        ; Class declarations
        (class_declaration
          name: (identifier) @class_name)

        ; Interface declarations
        (interface_declaration
          name: (identifier) @interface_name)

        ; Method declarations
        (method_declaration
          name: (identifier) @method_name)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_name)

        ; Parameters
        (formal_parameter
          name: (identifier) @param_name)

        ; USAGES
        (field_access field: (identifier) @field_usage)
        (method_invocation name: (identifier) @method_usage)
        (argument_list (identifier) @arg_usage)
        (assignment_expression left: (identifier) @var_usage)
        (assignment_expression right: (identifier) @var_usage)

        ; Type identifiers
        (type_identifier) @type_name
    "#;

    let query = Query::new(&tree.language(), query_text)
        .context("[determine_symbol_type_from_context] failed to create query")?;
    let mut cursor = QueryCursor::new();

    let mut found = false;

    let mut result = Err(anyhow!("[determine_symbol_type_from_context] invalid data"));

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found {
                return;
            }

            for capture in query_match.captures {
                let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap();
                if capture_text == node_text {
                    let capture_name = query.capture_names()[capture.index as usize];
                    let symbol = match capture_name {
                        "variable_name" => SymbolType::Variable,
                        "field_name" => SymbolType::Field,
                        "class_name" => SymbolType::Class,
                        "interface_name" => SymbolType::Interface,
                        "method_name" => SymbolType::Method,
                        "enum_name" => SymbolType::Enum,
                        "param_name" => SymbolType::Parameter,
                        "method_usage" => SymbolType::Function,
                        "type_name" => SymbolType::Type,
                        "field_usage" => SymbolType::Field,
                        _ => SymbolType::Variable,
                    };

                    result = Ok(symbol);
                    found = true;
                }
            }
        });

    result
}

fn check_complex_structures(node: &Node) -> Option<SymbolType> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "package_declaration" => {
                return Some(SymbolType::Package);
            }
            "scoped_identifier" => {
                if is_inside_package_declaration(&parent) {
                    return Some(SymbolType::Package);
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    None
}

fn is_inside_package_declaration(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "package_declaration" {
            return true;
        }
        current = parent.parent();
    }
    false
}

// pub fn find_definition_location_backup(
//     tree: &Tree,
//     source: &str,
//     dependency_cache: Arc<DependencyCache>,
//     file_uri: &str,
//     usage_node: &Node,
// ) -> Result<Location> {
//     // First search locally in current file
//     if let Some(local_location) = find_local(tree, source, file_uri, usage_node) {
//         return Ok(local_location);
//     }
//
//     // TODO: implement and test
//     // Check if it's a builtin type
//     // if let Some(builtin_location) = search_builtin_types(symbol, &dependency_cache) {
//     //     return Some(builtin_location);
//     // }
//
//     // Search in project dependencies
//     // Convert URI to file path to determine project root
//     let current_file_path = uri_to_path(file_uri).context(format!(
//         "[find_definition_location] failed to convert uri {} to path",
//         &file_uri
//     ))?;
//     let project_root = find_project_root(&current_file_path).context(format!(
//         "[find_definition_location] cannot find the project root. file_uri: {}",
//         &file_uri,
//     ))?;
//
//     // Look up symbol in the dependency cache
//     // The symbol_index maps (project_root, symbol_name) -> Vec<file_locations>
//     let symbol_name = usage_node.utf8_text(source.as_bytes()).context(format!(
//         "[find_definition_location] cannot get the symbol name for node {:#?}",
//         usage_node
//     ))?;
//     let symbol_key = (project_root.clone(), symbol_name.to_string());
//     let symbol_locations = dependency_cache
//         .symbol_index
//         .get(&symbol_key)
//         .context(format!(
//             "[find_definition_location] cannot get location for symbol key: {:#?}",
//             symbol_key
//         ))?;
//
//     // Search through each potential location
//     // There might be multiple files containing the same symbol name
//     for file_path in symbol_locations.iter() {
//         if let Some(external_location) = search_in_external_file_for_location(file_path, usage_node)
//         {
//             return Ok(external_location);
//         }
//     }
//
//     Err(anyhow!("[find_definition_location] invalid data"))
// }
