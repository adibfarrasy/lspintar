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
    let query_text = r#"(function_declaration name: (identifier) @name)"#;
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut best_match = None;
    let mut best_score = 0;
    let mut fallback_match = None; // For name-only matching when signature fails

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let name_node = capture.node;
                let name_text = name_node.utf8_text(source.as_bytes()).unwrap_or("");

                if name_text == method_name {
                    // CRITICAL: Only consider nodes that are actually method declaration names
                    if let Some(method_decl) = name_node.parent() {
                        if method_decl.kind() == "function_declaration" {
                            // Keep first method declaration as fallback
                            if fallback_match.is_none() {
                                fallback_match = Some(name_node);
                            }
                            
                            if let Some(method_sig) = extract_method_signature(&method_decl, source) {
                                let score = calculate_signature_match_score(call_signature, &method_sig);
                                if score > best_score {
                                    best_score = score;
                                    best_match = Some(name_node);
                                }
                            }
                        }
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
    extract_call_signature_from_invocation(&method_invocation, source)
}

#[tracing::instrument(skip_all)]
fn extract_call_signature_from_invocation(method_invocation: &Node, source: &str) -> Option<CallSignature> {
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
fn find_parent_method_invocation<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    while let Some(n) = current {
        if n.kind() == "method_invocation" {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(method_decl: &Node, source: &str) -> Option<MethodSignature> {
    let parameters = method_decl.child_by_field_name("parameters")?;
    
    let mut param_types = Vec::new();
    let mut cursor = parameters.walk();

    for child in parameters.named_children(&mut cursor) {
        if child.kind() == "parameter" {
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_text = type_node.utf8_text(source.as_bytes()).unwrap_or("Unknown");
                param_types.push(type_text.to_string());
            }
        }
    }

    Some(MethodSignature {
        param_count: param_types.len(),
        param_types,
    })
}

#[tracing::instrument(skip_all)]
pub fn calculate_signature_match_score(call_sig: &CallSignature, method_sig: &MethodSignature) -> u32 {
    let mut score = 0;

    // Perfect parameter count match gets highest score
    if call_sig.arg_count == method_sig.param_count {
        score += 100;
        
        // Check type compatibility for each parameter
        for (i, call_type) in call_sig.arg_types.iter().enumerate() {
            if let Some(method_param_type) = method_sig.param_types.get(i) {
                if let Some(call_type_str) = call_type {
                    if call_type_str == method_param_type {
                        score += 10; // Exact type match
                    } else if is_compatible_type(call_type_str, method_param_type) {
                        score += 5; // Compatible type match
                    }
                } else {
                    score += 1; // Unknown type, weak match
                }
            }
        }
    } else {
        // Penalize parameter count mismatch but still give some score for name match
        let count_diff = (call_sig.arg_count as i32 - method_sig.param_count as i32).abs();
        score += if count_diff <= 2 { 10 } else { 1 };
    }

    score
}

#[tracing::instrument(skip_all)]
fn infer_argument_type(arg_node: &Node, source: &str) -> Option<String> {
    match arg_node.kind() {
        "string_literal" => Some("String".to_string()),
        "decimal_integer_literal" => Some("int".to_string()),
        "decimal_floating_point_literal" => Some("double".to_string()),
        "true" | "false" => Some("boolean".to_string()),
        "null_literal" => Some("null".to_string()),
        "identifier" => {
            // Try to infer from variable name or context
            // This is a simplified heuristic
            let text = arg_node.utf8_text(source.as_bytes()).unwrap_or("");
            if text.ends_with("String") || text.contains("str") {
                Some("String".to_string())
            } else if text.ends_with("Count") || text.contains("num") {
                Some("int".to_string())
            } else {
                None // Cannot infer
            }
        },
        "method_invocation" => {
            // Try to infer return type from method name
            if let Some(method_name) = arg_node.child_by_field_name("name") {
                let name = method_name.utf8_text(source.as_bytes()).unwrap_or("");
                if name.starts_with("get") && name.contains("String") {
                    Some("String".to_string())
                } else if name.starts_with("get") && name.contains("Int") {
                    Some("int".to_string())
                } else {
                    None
                }
            } else {
                None
            }
        },
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
fn is_compatible_type(call_type: &str, param_type: &str) -> bool {
    // Simple type compatibility rules for Java
    match (call_type, param_type) {
        // Primitive widening conversions
        ("int", "long") | ("int", "float") | ("int", "double") => true,
        ("long", "float") | ("long", "double") => true,
        ("float", "double") => true,
        
        // Autoboxing/unboxing
        ("int", "Integer") | ("Integer", "int") => true,
        ("long", "Long") | ("Long", "long") => true,
        ("double", "Double") | ("Double", "double") => true,
        ("boolean", "Boolean") | ("Boolean", "boolean") => true,
        
        // Object hierarchy (simplified)
        (_, "Object") => true, // Everything extends Object
        ("null", _) => true,   // null can be assigned to any reference type
        
        _ => false,
    }
}