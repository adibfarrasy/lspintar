use std::usize;

use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::node_to_lsp_location},
    languages::LanguageSupport,
};

use super::utils::{
    find_definition_candidates, get_declaration_query_for_symbol_type, get_or_create_query,
};

#[tracing::instrument(skip_all)]
pub fn find_local(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let definition_node = search_local_definitions(tree, source, usage_node, language_support)?;

    node_to_lsp_location(&definition_node, file_uri)
}

#[tracing::instrument(skip_all)]
pub fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    language_support: &dyn LanguageSupport,
) -> Option<Node<'a>> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    let symbol_type = language_support
        .determine_symbol_type_from_context(tree, usage_node, source)
        .ok()?;

    if symbol_type.is_declaration() {
        return None;
    }

    match symbol_type {
        SymbolType::MethodCall | SymbolType::FunctionCall => {
            // Use specialized method resolution for method calls
            find_best_method_match(tree, source, usage_node, symbol_name)
        }
        SymbolType::VariableUsage => {
            // Search for variable declarations in accessible scopes
            let variable_candidates =
                find_variable_declarations_in_scope(tree, source, usage_node, &symbol_name);

            if !variable_candidates.is_empty() {
                // Find the best match using scope distance
                if let Some(best_match) = find_closest_declaration(usage_node, &variable_candidates)
                {
                    return Some(best_match);
                }
            }

            // Not found as variable, try as field
            find_as_field(tree, source, symbol_name)
        }

        SymbolType::FieldUsage => {
            // Get candidates for field usage
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;

            // For field usage (like autowired properties), find the field declaration
            // Fields are class-level, so we don't need closest scope logic
            candidates.into_iter().next()
        }

        _ => {
            // For other symbol types, use the general candidate finding approach
            let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;
            let candidates = find_definition_candidates(tree, source, &symbol_name, query_text)?;
            candidates.into_iter().next()
        }
    }
}

/// Find variable declarations that are accessible from the usage point
/// This includes:
/// 1. Variable declarations in the same block that come before usage
/// 2. Variable declarations in parent blocks
/// 3. Method parameters
fn find_variable_declarations_in_scope<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Vec<Node<'a>> {
    let mut candidates = Vec::new();

    // 1. Check method parameters first (highest priority)
    if let Some(method) = find_containing_method(usage_node) {
        let param_query = r#"(formal_parameter (identifier) @name)"#;
        let language = tree.language();

        if let Some(query) = get_or_create_query(param_query, &language) {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, method, source.as_bytes());

            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    if let Ok(param_text) = capture.node.utf8_text(source.as_bytes()) {
                        if param_text == symbol_name {
                            candidates.push(capture.node);
                        }
                    }
                }
            }
        }
    }

    // 2. Walk up from usage node to find variable declarations in accessible scopes
    let mut current_node = Some(*usage_node);

    while let Some(node) = current_node {
        // For each ancestor node, check its children for variable declarations
        // that come before the usage position
        if matches!(
            node.kind(),
            "block" | "method_declaration" | "class_declaration"
        ) {
            // Try multiple query patterns for different declaration types
            let queries = vec![
                r#"(variable_declaration) @decl"#, // Standard variable declarations with type
                r#"(expression_statement (identifier) @bare_id)"#,
            ];

            let language = tree.language();

            for query_text in queries {
                if let Some(query) = get_or_create_query(query_text, &language) {
                    let mut cursor = tree_sitter::QueryCursor::new();
                    let mut matches = cursor.matches(&query, node, source.as_bytes());

                    while let Some(query_match) = matches.next() {
                        for capture in query_match.captures {
                            let var_decl = capture.node;

                            // Check if this declaration contains our symbol
                            if let Ok(decl_text) = var_decl.utf8_text(source.as_bytes()) {
                                if decl_text.contains(symbol_name) {
                                    // Make sure declaration comes before usage (for same block)
                                    // or is in a parent block
                                    if var_decl.start_position() < usage_node.start_position() {
                                        // Handle different types of declarations
                                        if var_decl.kind() == "identifier"
                                            && decl_text == symbol_name
                                        {
                                            // For bare identifier declarations (expression_statement containing identifier)
                                            candidates.push(var_decl);
                                        } else {
                                            // Find the actual identifier node within the declaration
                                            if let Some(identifier) = find_identifier_in_declaration(
                                                &var_decl,
                                                source,
                                                symbol_name,
                                            ) {
                                                candidates.push(identifier);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        current_node = node.parent();
    }

    candidates
}

/// Find the identifier node within a variable declaration that matches the symbol name
fn find_identifier_in_declaration<'a>(
    var_decl: &Node<'a>,
    source: &str,
    symbol_name: &str,
) -> Option<Node<'a>> {
    let mut cursor = var_decl.walk();

    for child in var_decl.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            // Look for the identifier in the variable declarator
            let mut declarator_cursor = child.walk();
            for declarator_child in child.children(&mut declarator_cursor) {
                if declarator_child.kind() == "identifier" {
                    if let Ok(id_text) = declarator_child.utf8_text(source.as_bytes()) {
                        if id_text == symbol_name {
                            return Some(declarator_child);
                        }
                    }
                }
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn find_closest_declaration<'a>(usage_node: &Node, candidates: &[Node<'a>]) -> Option<Node<'a>> {
    let mut best_candidate = None;
    let mut best_scope_distance = usize::MAX;

    for candidate in candidates.iter() {
        if let Some(distance) = calculate_scope_distance(usage_node, candidate) {
            if distance < best_scope_distance {
                best_scope_distance = distance;
                best_candidate = Some(*candidate);
            }
        }
    }

    best_candidate
}

#[tracing::instrument(skip_all)]
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
    let decl_method = find_containing_method(declaration_node);
    let usage_method = find_containing_method(usage_node);
    let decl_block = find_containing_block(declaration_node);
    let usage_block = find_containing_block(usage_node);

    // For formal parameters, check if usage is in the same method
    if let Some(decl_method) = decl_method {
        if let Some(usage_method) = usage_method {
            return decl_method.id() == usage_method.id();
        }
    }

    // For local variables, check if declaration comes before usage in same block
    if let Some(decl_block) = decl_block {
        if let Some(usage_block) = usage_block {
            if decl_block.id() == usage_block.id() {
                return declaration_node.start_position() < usage_node.start_position();
            }
        }
    }

    // Handle top-level declarations: if declaration has no containing block,
    // it's accessible from any nested scope as long as it comes before usage
    if decl_block.is_none() {
        // Declaration is at top level, check if it comes before usage
        if declaration_node.start_position() < usage_node.start_position() {
            // Additional check: make sure they're in the same top-level context
            if let Some(usage_method) = usage_method {
                // Usage is inside a method, declaration should be either:
                // 1. A parameter of the same method, or
                // 2. A top-level declaration accessible to that method
                if let Some(decl_method) = find_containing_method(declaration_node) {
                    // Both are in methods - must be same method for parameters
                    return decl_method.id() == usage_method.id();
                } else {
                    // Declaration is at class/file level, usage is in method - accessible
                    return true;
                }
            } else {
                // Both are at the same level (class/file level)
                return true;
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

#[tracing::instrument(skip_all)]
fn extract_call_signature(usage_node: &Node, source: &str) -> Option<CallSignature> {
    let method_invocation = find_parent_method_invocation(usage_node)?;

    let arguments = method_invocation.child_by_field_name("arguments")?;

    let mut arg_types = Vec::new();
    let mut cursor = arguments.walk();

    for child in arguments.named_children(&mut cursor) {
        let arg_type = infer_argument_type(&child, source);
        arg_types.push(arg_type);
    }

    Some(CallSignature {
        arg_count: arg_types.len(),
        arg_types,
    })
}

#[tracing::instrument(skip_all)]
fn extract_method_signature(method_node: &Node, source: &str) -> Option<MethodSignature> {
    if method_node.kind() != "method_declaration" {
        return None;
    }

    let parameters = method_node.child_by_field_name("parameters")?;

    let mut param_types = Vec::new();
    let mut param_names = Vec::new();
    let mut cursor = parameters.walk();

    let mut has_spread = false;

    for child in parameters.named_children(&mut cursor) {
        if vec!["formal_parameter", "spread_parameter"].contains(&child.kind()) {
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
                // TODO: Could enhance with variable type lookup
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

        // Collection interface compatibility
        ("List", "Collection") => true,
        ("Map", "Object") => true,
        ("List", "Object") => true,

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

#[tracing::instrument(skip_all)]
fn find_best_method_match<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
    symbol_name: &str,
) -> Option<Node<'a>> {
    let query_text = r#"(method_declaration name: (identifier) @name)"#;
    let query = get_or_create_query(query_text, &tree.language())?;
    let mut cursor = QueryCursor::new();

    let mut best_match = None;
    let call_signature = extract_call_signature(usage_node, source);

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    'outer: while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let name_node = capture.node;
            let name_text = name_node.utf8_text(source.as_bytes()).unwrap_or("");

            if name_text == symbol_name {
                // If we have a call signature, try to match by signature
                if let Some(ref call_sig) = call_signature {
                    if let Some(method_decl) = name_node.parent() {
                        if let Some(method_sig) = extract_method_signature(&method_decl, source) {
                            if signatures_match(call_sig, &method_sig) {
                                best_match = Some(name_node);
                                break 'outer;
                            }
                        }
                    }
                } else {
                    // Fallback: if we can't extract call signature, just match by name
                    best_match = Some(name_node);
                    break 'outer;
                }
            }
        }
    }

    best_match
}

/// Try to find a symbol as a field declaration in the current class
fn find_as_field<'a>(tree: &'a Tree, source: &str, symbol_name: &str) -> Option<Node<'a>> {
    let field_query_text =
        r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#;
    let query = get_or_create_query(field_query_text, &tree.language())?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == symbol_name {
                    return Some(capture.node);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::utils::create_parser_for_language;
    use crate::languages::groovy::support::GroovySupport;
    use tower_lsp::lsp_types::Position;
    use tree_sitter::Tree;

    struct VariableDefinitionTestCase {
        name: &'static str,
        source_code: &'static str,
        usage_position: Position, // Position of variable usage
        expected_declaration_text: &'static str, // Expected text of declaration
        should_find_definition: bool,
    }

    fn create_test_tree(source: &str) -> Tree {
        let mut parser = create_parser_for_language("groovy").unwrap();
        parser.parse(source, None).unwrap()
    }

    fn find_node_at_position<'a>(
        tree: &'a Tree,
        source: &str,
        position: Position,
    ) -> Option<tree_sitter::Node<'a>> {
        let target_byte = position_to_byte_offset(source, position)?;
        let mut current = tree.root_node();

        loop {
            let mut found_child = None;
            let mut cursor = current.walk();

            for child in current.children(&mut cursor) {
                if child.start_byte() <= target_byte && target_byte <= child.end_byte() {
                    found_child = Some(child);
                    break;
                }
            }

            match found_child {
                Some(child) => current = child,
                None => break,
            }
        }

        Some(current)
    }

    fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
        let mut byte_offset = 0;
        let mut line = 0;
        let mut column = 0;

        for ch in source.chars() {
            if line == position.line as usize && column == position.character as usize {
                return Some(byte_offset);
            }

            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }

            byte_offset += ch.len_utf8();
        }

        None
    }
}
