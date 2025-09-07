//! Simple type inference for Groovy to improve IDE experience
//!
//! This provides basic type hints for common patterns like:
//! - `def x = "hello"` → String  
//! - `def list = [1, 2, 3]` → List<Integer>
//! - `def map = [a: 1, b: 2]` → Map<String, Integer>

use crate::types::Position;
use crate::core::types::TypeHint;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

/// Infer a type hint for a variable at the given position
pub fn infer_variable_type(source: &str, position: Position) -> Option<TypeHint> {
    let mut parser = create_groovy_parser()?;
    let tree = parser.parse(source, None)?;
    
    let variable_declaration = find_variable_declaration_at_position(&tree, source, position)?;
    if let Some(initializer) = variable_declaration.child_by_field_name("value") {
        return infer_expression_type(&initializer, source);
    }
    
    None
}

fn create_groovy_parser() -> Option<Parser> {
    let mut parser = Parser::new();
    let language = tree_sitter_groovy::language();
    parser.set_language(&language).ok()?;
    Some(parser)
}

fn find_variable_declaration_at_position<'a>(
    tree: &'a Tree,
    source: &str,
    position: Position,
) -> Option<Node<'a>> {
    let query_text = r#"
    (variable_declarator
      name: (identifier) @name
      value: (_) @value) @declaration
    "#;
    
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(match_) = matches.next() {
        let declaration_node = match_.captures.iter()
            .find(|c| query.capture_names()[c.index as usize] == "declaration")?
            .node;
            
        let name_node = match_.captures.iter()
            .find(|c| query.capture_names()[c.index as usize] == "name")?
            .node;
            
        if node_contains_position(&name_node, position) {
            return Some(declaration_node);
        }
    }
    
    None
}

fn node_contains_position(node: &Node, position: Position) -> bool {
    let start_position = node.start_position();
    let end_position = node.end_position();
    
    let pos_line = position.line as usize;
    let pos_char = position.character as usize;
    
    if pos_line < start_position.row || pos_line > end_position.row {
        return false;
    }
    
    if pos_line == start_position.row && pos_char < start_position.column {
        return false;
    }
    
    if pos_line == end_position.row && pos_char > end_position.column {
        return false;
    }
    
    true
}

/// Infer type hint from an expression node
fn infer_expression_type(node: &Node, source: &str) -> Option<TypeHint> {
    match node.kind() {
        "string_literal" => Some(TypeHint::groovy_string()),
        "decimal_integer_literal" => {
            // Check if it's a Long literal (ends with L)
            let text = node.utf8_text(source.as_bytes()).ok()?;
            if text.ends_with('L') || text.ends_with('l') {
                Some(TypeHint::groovy_long())
            } else {
                Some(TypeHint::groovy_integer())
            }
        },
        "decimal_floating_point_literal" => {
            // Check for explicit type suffixes
            let text = node.utf8_text(source.as_bytes()).ok()?;
            if text.ends_with('D') || text.ends_with('d') {
                Some(TypeHint::groovy_double())
            } else if text.ends_with('F') || text.ends_with('f') {
                Some(TypeHint::groovy_float())
            } else {
                // Default for decimal literals in Groovy is BigDecimal
                Some(TypeHint::groovy_bigdecimal())
            }
        },
        "true" | "false" => Some(TypeHint::groovy_boolean()),
        "null_literal" => Some(TypeHint::groovy_unknown()),
        
        "array_literal" => {
            // Try to infer element types from list elements
            let element_hint = infer_list_element_type(node, source);
            Some(TypeHint::groovy_list(&element_hint))
        },
        
        "map_literal" => {
            // Infer key and value types from map entries
            let (key_hint, value_hint) = infer_map_types(node, source);
            Some(TypeHint::groovy_map(&key_hint, &value_hint))
        },
        
        "object_creation_expression" => infer_constructor_type(node, source),
        
        "method_call" => infer_method_return_type(node, source),
        
        _ => Some(TypeHint::groovy_unknown()),
    }
}

/// Infer element type from list literal by analyzing all elements
fn infer_list_element_type(list_node: &Node, source: &str) -> String {
    let mut element_types = Vec::new();
    
    // Collect types from all elements (skip brackets and commas)
    for i in 0..list_node.child_count() {
        if let Some(child) = list_node.child(i) {
            if child.kind() != "[" && child.kind() != "]" && child.kind() != "," {
                if let Some(hint) = infer_expression_type(&child, source) {
                    element_types.push(hint.display_name);
                }
            }
        }
    }
    
    if element_types.is_empty() {
        return "Object".to_string();
    }
    
    // If all elements have the same type, use that
    let first_type = &element_types[0];
    if element_types.iter().all(|t| t == first_type) {
        return first_type.clone();
    }
    
    // If we have mixed numeric types, try to find common numeric type
    if element_types.iter().all(|t| is_numeric_type(t)) {
        return infer_common_numeric_type(&element_types);
    }
    
    // Otherwise, use Object as the common supertype
    "Object".to_string()
}


/// Infer key and value types from map literal entries
fn infer_map_types(map_node: &Node, source: &str) -> (String, String) {
    let mut key_types = Vec::new();
    let mut value_types = Vec::new();
    
    // Look for map_entry nodes
    for i in 0..map_node.child_count() {
        if let Some(child) = map_node.child(i) {
            if child.kind() == "map_entry" {
                // Extract key type
                if let Some(key_node) = child.child_by_field_name("key") {
                    // map_key contains the actual key expression
                    if let Some(key_expr) = key_node.child(0) {
                        let key_type = if key_expr.kind() == "identifier" {
                            // In Groovy, identifier keys in map literals are treated as strings
                            "String".to_string()
                        } else if let Some(key_hint) = infer_expression_type(&key_expr, source) {
                            key_hint.display_name
                        } else {
                            "Object".to_string()
                        };
                        key_types.push(key_type);
                    }
                }
                
                // Extract value type  
                if let Some(value_node) = child.child_by_field_name("value") {
                    if let Some(value_hint) = infer_expression_type(&value_node, source) {
                        value_types.push(value_hint.display_name);
                    }
                }
            }
        }
    }
    
    let key_type = infer_common_type(&key_types);
    let value_type = infer_common_type(&value_types);
    
    (key_type, value_type)
}

/// Find common type for a collection of types
fn infer_common_type(types: &[String]) -> String {
    if types.is_empty() {
        return "Object".to_string();
    }
    
    let first_type = &types[0];
    if types.iter().all(|t| t == first_type) {
        return first_type.clone();
    }
    
    // Handle numeric types specially
    if types.iter().all(|t| is_numeric_type(t)) {
        return infer_common_numeric_type(types);
    }
    
    "Object".to_string()
}

/// Check if a type is numeric
fn is_numeric_type(type_name: &str) -> bool {
    matches!(type_name, "Integer" | "Long" | "Float" | "Double" | "BigDecimal")
}

/// Infer common numeric type (following Groovy's type promotion rules)
fn infer_common_numeric_type(types: &[String]) -> String {
    let mut has_bigdecimal = false;
    let mut has_double = false;
    let mut has_float = false;
    let mut has_long = false;
    
    for t in types {
        match t.as_str() {
            "BigDecimal" => has_bigdecimal = true,
            "Double" => has_double = true,
            "Float" => has_float = true,
            "Long" => has_long = true,
            _ => {} // Integer is the base case
        }
    }
    
    // Groovy type promotion: BigDecimal > Double > Float > Long > Integer
    if has_bigdecimal {
        "BigDecimal".to_string()
    } else if has_double {
        "Double".to_string()
    } else if has_float {
        "Float".to_string()
    } else if has_long {
        "Long".to_string()
    } else {
        "Integer".to_string()
    }
}

/// Infer type from constructor call like `new ArrayList<String>()` or `new File("path")`
fn infer_constructor_type(node: &Node, source: &str) -> Option<TypeHint> {
    if let Some(type_node) = node.child_by_field_name("type") {
        match type_node.kind() {
            "generic_type" => {
                // Handle generic types like ArrayList<String>
                if let Some(base_type) = type_node.child(0) {
                    let base_name = base_type.utf8_text(source.as_bytes()).ok()?;
                    
                    // Extract generic type arguments - look for type_arguments child
                    let mut type_args_node = None;
                    for i in 0..type_node.child_count() {
                        if let Some(child) = type_node.child(i) {
                            if child.kind() == "type_arguments" {
                                type_args_node = Some(child);
                                break;
                            }
                        }
                    }
                    
                    if let Some(type_args_node) = type_args_node {
                        let generic_args = extract_generic_arguments(&type_args_node, source);
                        
                        match base_name {
                            "ArrayList" | "List" => {
                                let element_type = generic_args.first().map(|s| s.as_str()).unwrap_or("Object");
                                Some(TypeHint::groovy_list(element_type))
                            },
                            "HashMap" | "Map" => {
                                let key_type = generic_args.first().map(|s| s.as_str()).unwrap_or("Object");
                                let value_type = generic_args.get(1).map(|s| s.as_str()).unwrap_or("Object");
                                Some(TypeHint::groovy_map(key_type, value_type))
                            },
                            _ => {
                                // Generic type we don't handle specially
                                let full_type = type_node.utf8_text(source.as_bytes()).ok()?;
                                Some(TypeHint::likely(full_type))
                            }
                        }
                    } else {
                        // Generic type without arguments
                        match base_name {
                            "ArrayList" => Some(TypeHint::groovy_list("Object")),
                            "HashMap" => Some(TypeHint::groovy_map("Object", "Object")),
                            _ => Some(TypeHint::likely(base_name)),
                        }
                    }
                } else {
                    Some(TypeHint::groovy_unknown())
                }
            },
            "type_identifier" => {
                // Handle simple types like File, String, etc.
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                
                match type_text {
                    "String" => Some(TypeHint::groovy_string()),
                    "Integer" => Some(TypeHint::groovy_integer()),
                    "Boolean" => Some(TypeHint::groovy_boolean()),
                    "ArrayList" => Some(TypeHint::groovy_list("Object")),
                    "HashMap" => Some(TypeHint::groovy_map("Object", "Object")),
                    _ => {
                        // Create a qualified type hint for known Java classes
                        let qualified_name = match type_text {
                            "File" => Some("java.io.File".to_string()),
                            "Date" => Some("java.util.Date".to_string()),
                            "StringBuilder" => Some("java.lang.StringBuilder".to_string()),
                            "StringBuffer" => Some("java.lang.StringBuffer".to_string()),
                            _ => None,
                        };
                        
                        Some(TypeHint {
                            display_name: type_text.to_string(),
                            qualified_name,
                            confidence: crate::core::types::Confidence::High,
                        })
                    }
                }
            },
            _ => {
                // Fallback for other type node kinds
                let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                Some(TypeHint::likely(type_text))
            }
        }
    } else {
        Some(TypeHint::groovy_unknown())
    }
}

/// Extract generic type arguments from type_arguments node
fn extract_generic_arguments(type_args_node: &Node, source: &str) -> Vec<String> {
    let mut args = Vec::new();
    
    for i in 0..type_args_node.child_count() {
        if let Some(child) = type_args_node.child(i) {
            if child.kind() == "type_identifier" {
                if let Ok(arg_text) = child.utf8_text(source.as_bytes()) {
                    args.push(arg_text.to_string());
                }
            }
        }
    }
    
    args
}

/// Infer return type from method call
fn infer_method_return_type(node: &Node, source: &str) -> Option<TypeHint> {
    let method_name = node.child_by_field_name("name")?
        .utf8_text(source.as_bytes()).ok()?;
        
    match method_name {
        "size" | "length" => Some(TypeHint::groovy_integer()),
        "toString" => Some(TypeHint::groovy_string()),
        "isEmpty" => Some(TypeHint::groovy_boolean()),
        _ => Some(TypeHint::groovy_unknown()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_literal_inference() {
        // Test basic type inference with actual parsed nodes
        let test_cases = vec![
            ("\"hello\"", "String"),
            ("42", "Integer"), 
            ("42L", "Long"),
            ("42l", "Long"),
            ("3.14", "BigDecimal"), // Groovy default for decimal literals
            ("3.14D", "Double"),
            ("3.14d", "Double"),
            ("3.14F", "Float"),
            ("3.14f", "Float"),
            ("true", "Boolean"),
            ("false", "Boolean"),
        ];
        
        for (source, expected) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let root = tree.root_node();
                    // Navigate: source_file -> expression_statement -> literal
                    if let Some(expr_stmt) = root.child(0) {
                        if let Some(literal) = expr_stmt.child(0) {
                            if let Some(hint) = infer_expression_type(&literal, source) {
                                assert_eq!(hint.display_name, expected);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_array_literal_inference() {
        let test_cases = vec![
            ("[1, 2, 3]", "List<Integer>"),
            ("[1, 2L, 3]", "List<Long>"), // Mixed integer types -> Long
            ("[1.0, 2.5]", "List<BigDecimal>"), // Default Groovy decimal
            ("[1.0F, 2.5F]", "List<Float>"),
            ("[1.0D, 2.5D]", "List<Double>"),
            (r#"["a", "b", "c"]"#, "List<String>"),
            ("[true, false]", "List<Boolean>"),
            (r#"[1, "hello"]"#, "List<Object>"), // Mixed types -> Object
        ];
        
        for (source, expected) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let root = tree.root_node();
                    if let Some(expr_stmt) = root.child(0) {
                        if let Some(array_literal) = expr_stmt.child(0) {
                            if let Some(hint) = infer_expression_type(&array_literal, source) {
                                assert_eq!(hint.display_name, expected, "Failed for: {}", source);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_map_literal_inference() {
        let test_cases = vec![
            (r#"[a: 1, b: 2]"#, "Map<String, Integer>"),
            (r#"["key1": 1, "key2": 2]"#, "Map<String, Integer>"),
            (r#"[1: "value1", 2: "value2"]"#, "Map<Integer, String>"),
            (r#"[a: 1, b: "hello"]"#, "Map<String, Object>"), // Mixed value types
            (r#"[1: 2, "key": 3]"#, "Map<Object, Integer>"), // Mixed key types
        ];
        
        for (source, expected) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let root = tree.root_node();
                    if let Some(expr_stmt) = root.child(0) {
                        if let Some(map_literal) = expr_stmt.child(0) {
                            if let Some(hint) = infer_expression_type(&map_literal, source) {
                                assert_eq!(hint.display_name, expected, "Failed for: {}", source);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test] 
    fn test_constructor_inference() {
        let test_cases = vec![
            (r#"new String("hello")"#, "String"),
            (r#"new File("/path")"#, "File"),
            (r#"new ArrayList<String>()"#, "List<String>"),
            (r#"new HashMap<String, Integer>()"#, "Map<String, Integer>"),
            (r#"new ArrayList()"#, "List<Object>"), // No generics -> Object
            (r#"new Date()"#, "Date"),
            (r#"new StringBuilder()"#, "StringBuilder"),
        ];
        
        for (source, expected) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let root = tree.root_node();
                    if let Some(expr_stmt) = root.child(0) {
                        if let Some(constructor) = expr_stmt.child(0) {
                            if let Some(hint) = infer_expression_type(&constructor, source) {
                                assert_eq!(hint.display_name, expected, "Failed for: {}", source);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_numeric_type_promotion() {
        assert_eq!(infer_common_numeric_type(&["Integer".to_string(), "Long".to_string()]), "Long");
        assert_eq!(infer_common_numeric_type(&["Integer".to_string(), "Float".to_string()]), "Float");
        assert_eq!(infer_common_numeric_type(&["Long".to_string(), "Double".to_string()]), "Double");
        assert_eq!(infer_common_numeric_type(&["Float".to_string(), "BigDecimal".to_string()]), "BigDecimal");
        assert_eq!(infer_common_numeric_type(&["Integer".to_string(), "Integer".to_string()]), "Integer");
    }

    #[test]
    fn test_variable_type_inference_end_to_end() {
        let test_cases = vec![
            ("def a = 42", Position { line: 0, character: 4 }, Some("Integer")),
            ("def b = \"hello\"", Position { line: 0, character: 4 }, Some("String")),  
            ("def c = [1, 2, 3]", Position { line: 0, character: 4 }, Some("List<Integer>")),
            ("def d = [a: 1, b: 2]", Position { line: 0, character: 4 }, Some("Map<String, Integer>")),
            ("def e = [\"a\": 1, \"b\": 2]", Position { line: 0, character: 4 }, Some("Map<String, Integer>")),
            ("def f = new File(\"/path\")", Position { line: 0, character: 4 }, Some("File")),
        ];
        
        for (source, position, expected) in test_cases {
            let result = infer_variable_type(source, position);
            
            match (result, expected) {
                (Some(hint), Some(expected_type)) => {
                    assert_eq!(hint.display_name, expected_type, "Failed for: {}", source);
                },
                (None, None) => {}, // Expected no inference
                (got, expected) => panic!("Mismatch for '{}': got {:?}, expected {:?}", source, got, expected),
            }
        }
    }
}