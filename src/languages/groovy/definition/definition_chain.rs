use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

#[derive(Debug, Clone)]
pub struct CallSignature {
    pub arg_count: usize,
    pub arg_types: Vec<Option<String>>, // None if type can't be inferred
}

#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub param_count: usize,
    pub param_types: Vec<String>,
}

/// Enhanced method resolution that finds the best matching method based on call signature
#[tracing::instrument(skip_all)]
pub fn find_method_with_signature<'a>(
    tree: &'a Tree,
    source: &str,
    method_name: &str,
    call_signature: &CallSignature,
) -> Option<Node<'a>> {
    
    let query_text = r#"(method_declaration name: (identifier) @name)"#;
    
    let query = Query::new(&tree.language(), query_text);
    if query.is_err() {
        return None;
    }
    let query = query.unwrap();
    let mut cursor = QueryCursor::new();

    let mut best_match = None;
    let mut best_score = 0;
    let mut fallback_match = None; // For name-only matching when signature fails
    let mut candidate_count = 0;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let name_node = capture.node;
                let name_text = name_node.utf8_text(source.as_bytes()).unwrap_or("");

                if name_text == method_name {
                    candidate_count += 1;
                    
                    // Always keep the first name match as fallback
                    if fallback_match.is_none() {
                        fallback_match = Some(name_node);
                    }
                    
                    // Walk up the tree to find the method declaration
                    if let Some(method_decl) = find_method_declaration_ancestor(&name_node) {
                        if let Some(method_sig) = extract_method_signature(&method_decl, source) {
                            let score = calculate_signature_match_score(call_signature, &method_sig);
                            
                            if score > best_score {
                                best_score = score;
                                best_match = Some(name_node);
                            }
                        } else {
                        }
                    } else {
                    }
                }
            }
        });

    // If we have a signature match, use it; otherwise fall back to name match
    if best_score > 0 {
        best_match
    } else {
        fallback_match
    }
}

/// Extract call signature from method invocation context
#[tracing::instrument(skip_all)]
pub fn extract_call_signature_from_context(usage_node: &Node, source: &str) -> Option<CallSignature> {
    
    let method_invocation = find_parent_method_invocation(usage_node)?;
    
    let result = extract_call_signature_from_invocation(&method_invocation, source);
    result
}

#[tracing::instrument(skip_all)]
fn extract_call_signature_from_invocation(method_invocation: &Node, source: &str) -> Option<CallSignature> {
    
    let arguments = method_invocation.child_by_field_name("arguments");
    if arguments.is_none() {
        return None;
    }
    let arguments = arguments.unwrap();

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
    })
}

#[tracing::instrument(skip_all)]
fn calculate_signature_match_score(call_sig: &CallSignature, method_sig: &MethodSignature) -> u32 {
    
    // If parameter counts don't match and method doesn't have varargs, no match
    if call_sig.arg_count != method_sig.param_count && method_sig.param_count < usize::MAX {
        return 0;
    }

    let mut score = 100; // Base score for matching parameter count

    // Score based on type compatibility
    for (i, call_arg_type) in call_sig.arg_types.iter().enumerate() {
        if let Some(method_param_type) = method_sig.param_types.get(i) {
            if let Some(call_type) = call_arg_type {
                if types_compatible(call_type, method_param_type) {
                    let type_score = if call_type == method_param_type { 10 } else { 5 };
                    score += type_score;
                } else {
                    return 0; // Incompatible types
                }
            } else {
                score += 1; // Unknown type, small bonus for having a parameter
            }
        }
    }

    score
}

#[tracing::instrument(skip_all)]
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

/// Find the method declaration ancestor by walking up the tree
#[tracing::instrument(skip_all)]
fn find_method_declaration_ancestor<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    while let Some(curr_node) = current {
        if curr_node.kind() == "method_declaration" {
            return Some(curr_node);
        }
        current = curr_node.parent();
    }
    None
}

#[tracing::instrument(skip_all)]
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
        "text_block" => Some("String".to_string()),

        // Null literal
        "null_literal" => Some("null".to_string()),

        // Collection literals
        "map_literal" => Some("Map".to_string()),
        "array_literal" => Some("List".to_string()),

        // Complex expressions
        "identifier" => None,
        "method_invocation" => None,
        "field_access" => None,
        "cast_expression" => {
            if let Some(type_node) = arg_node.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(type_text.to_string())
            } else {
                None
            }
        }
        "parenthesized_expression" => {
            if let Some(inner_expr) = arg_node.child_by_field_name("expression") {
                infer_argument_type(&inner_expr, source)
            } else {
                None
            }
        }
        "object_creation_expression" => {
            if let Some(type_node) = arg_node.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(type_text.to_string())
            } else {
                None
            }
        }
        "binary_expression" => {
            if let Some(operator) = arg_node.child_by_field_name("operator") {
                let op_text = operator.utf8_text(source.as_bytes()).ok()?;
                match op_text {
                    "+" | "-" | "*" | "/" | "%" => {
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
        "ternary_expression" => {
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

#[tracing::instrument(skip_all)]
fn contains_floating_point_operand(binary_expr: &Node, _source: &str) -> bool {
    let mut cursor = binary_expr.walk();
    for child in binary_expr.children(&mut cursor) {
        match child.kind() {
            "decimal_floating_point_literal" | "hex_floating_point_literal" => return true,
            _ => continue,
        }
    }
    false
}

#[tracing::instrument(skip_all)]
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

#[tracing::instrument(skip_all)]
fn is_primitive_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "int" | "double" | "boolean" | "char" | "long" | "float" | "byte" | "short"
    )
}