use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use method::extract_method_signature;
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::{LanguageSupport, groovy::type_inference::infer_variable_type},
    types::Position as LspPosition,
};

mod class;
mod field;
mod interface;
mod method;

#[tracing::instrument(skip_all)]
pub fn handle(
    tree: &Tree,
    source: &str,
    location: Location,
    language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    let node = location_to_node(&location, tree);
    if node.is_none() {
        return None;
    }
    let node = node?;

    let symbol_type = language_support.determine_symbol_type_from_context(tree, &node, source);
    if symbol_type.is_err() {
        return None;
    }
    let symbol_type = symbol_type.ok()?;

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, &node, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, &node, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            match node.kind() {
                "class_declaration" => extract_class_signature(tree, &node, source),
                "interface_declaration" => extract_interface_signature(tree, &node, source),
                "enum_declaration" => {
                    // We don't have enum extraction yet, fall back to class
                    extract_class_signature(tree, &node, source)
                }
                _ => extract_class_signature(tree, &node, source)
            }
        }
        SymbolType::MethodCall => {
            // For method calls, try to find the declaration first, then extract signature
            if let Some(method_decl_node) = find_function_declaration_for_call(tree, &node, source) {
                extract_method_signature(tree, &method_decl_node, source)
            } else {
                // Fallback: provide basic method call info
                extract_method_call_info(&node, source)
            }
        }
        SymbolType::VariableDeclaration | SymbolType::VariableUsage => {
            extract_variable_info(tree, &node, source, &location)
        }
        _ => None
    };


    content.and_then(|c| {
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: c,
            }),
            range: Some(location.range),
        })
    })
}

/// Find method declaration for a method call within the same file
#[tracing::instrument(skip_all)]
fn find_function_declaration_for_call<'a>(
    tree: &'a Tree,
    node: &Node,
    source: &str,
) -> Option<Node<'a>> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    let query_text = r#"
        (function_declaration
          name: (identifier) @method_name
        )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(capture_text) = capture.node.utf8_text(source.as_bytes()) {
                if capture_text == method_name {
                    // Return the method declaration node (parent of identifier)
                    return capture.node.parent();
                }
            }
        }
    }

    None
}

/// Extract variable information with type inference for variables without explicit types
#[tracing::instrument(skip_all)]
fn extract_variable_info(_tree: &Tree, node: &Node, source: &str, location: &Location) -> Option<String> {
    // Try to find the variable declaration
    let var_decl_node = find_parent_of_kind(node, "variable_declaration")
        .or_else(|| find_parent_of_kind(node, "field_declaration"));

    if let Some(var_node) = var_decl_node {
        if let Ok(var_text) = var_node.utf8_text(source.as_bytes()) {
            let var_text = var_text.trim();
            
            // Check if the variable has an explicit type annotation
            // This is a simple heuristic: look for ': Type' pattern after 'def varname'
            let has_explicit_type = {
                // Look for pattern like "def varname: Type" 
                // We need to distinguish from map literals like [key: value]
                if let Some(equals_pos) = var_text.find('=') {
                    // Check if there's a colon before the equals sign (type annotation)
                    var_text[..equals_pos].contains(':') && !var_text[..equals_pos].contains('[')
                } else {
                    // No assignment, check if it's just a type declaration
                    var_text.contains(':')
                }
            };
            
            if !has_explicit_type {
                // Try to infer the type for variables without explicit types
                let position = LspPosition {
                    line: location.range.start.line,
                    character: location.range.start.character,
                };
                
                if let Some(type_hint) = infer_variable_type(source, position) {
                    return Some(format!(
                        "```groovy\n{}\n```\n\n*Inferred type: `{}`*", 
                        var_text, 
                        type_hint.display_name
                    ));
                }
            }
            
            // Fallback: just show the variable declaration
            return Some(format!("```groovy\n{}\n```", var_text));
        }
    }
    
    // If we can't find a declaration, try to provide basic variable info
    if let Ok(var_name) = node.utf8_text(source.as_bytes()) {
        Some(format!("```groovy\n{}\n```\n\n*Variable reference*", var_name))
    } else {
        None
    }
}

/// Find a parent node of a specific kind
fn find_parent_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut current = Some(*node);
    while let Some(node) = current {
        if node.kind() == kind {
            return Some(node);
        }
        current = node.parent();
    }
    None
}

/// Provide basic method call information when declaration can't be found
#[tracing::instrument(skip_all)]
fn extract_method_call_info(node: &Node, source: &str) -> Option<String> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    // Try to find the method invocation parent to get call context
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_invocation" {
            // Extract object and arguments if available
            let object_text = parent
                .child_by_field_name("object")
                .and_then(|obj| obj.utf8_text(source.as_bytes()).ok())
                .unwrap_or("");

            let args_text = parent
                .child_by_field_name("arguments")
                .and_then(|args| args.utf8_text(source.as_bytes()).ok())
                .unwrap_or("()");

            let call_info = if !object_text.is_empty() {
                format!("```groovy\n{}.{}{}\n```\n\n*Method call - definition not found in current file*", 
                       object_text, method_name, args_text)
            } else {
                format!(
                    "```groovy\n{}{}\n```\n\n*Method call - definition not found in current file*",
                    method_name, args_text
                )
            };

            return Some(call_info);
        }
        current = parent.parent();
    }

    // Fallback for standalone method name
    Some(format!(
        "```groovy\n{}\n```\n\n*Method reference*",
        method_name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Position, Range};
    use crate::core::utils::create_parser_for_language;

    fn create_groovy_parser() -> Option<tree_sitter::Parser> {
        create_parser_for_language("groovy")
    }

    #[test]
    fn test_hover_with_type_inference() {
        let test_cases = vec![
            // Variables without explicit types should show inferred types
            ("def a = 42", 0, 4, Some("Integer")),                    // def a = 42 // Integer
            ("def b = \"hello\"", 0, 4, Some("String")),             // def b = "hello" // String  
            ("def c = 4.2", 0, 4, Some("BigDecimal")),              // def c = 4.2 // BigDecimal
            ("def d = 42L", 0, 4, Some("Long")),                     // def d = 42L // Long
            ("def e = true", 0, 4, Some("Boolean")),                 // def e = true // Boolean
            ("def f = [\"abc\"]", 0, 4, Some("List<String>")),       // def f = ["abc"] // List<String>
            ("def g = [a: 1, b: 2]", 0, 4, Some("Map<String, Integer>")), // def g = [a: 1, b: 2] // Map<String, Integer>  
            ("def h = new ArrayList()", 0, 4, Some("List<Object>")),    // def h = new ArrayList() // List<Object>
            ("def i = new File(\"path\")", 0, 4, Some("File")),         // def i = new File("path") // File
            ("def j = [1, 2, 3]", 0, 4, Some("List<Integer>")),        // def j = [1, 2, 3] // List<Integer>
            ("def k = [a: 1, b: 2]", 0, 4, Some("Map<String, Integer>")), // def k = [a: 1, b: 2] // Map<String, Integer>
            
            // Variables with explicit types should NOT show inferred types (just the declaration)
            ("def a: String = \"hello\"", 0, 4, None),         // explicit type, no inference needed
            ("def b: Integer = 42", 0, 4, None),               // explicit type, no inference needed
        ];

        for (source, line, character, expected_inferred_type) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let location = Location {
                        uri: "file:///test.groovy".parse().unwrap(),
                        range: Range {
                            start: Position { line, character },
                            end: Position { line, character: character + 1 },
                        },
                    };

                    // Create a mock language support - for now just test the helper functions directly
                    let node = location_to_node(&location, &tree);
                    if let Some(node) = node {
                        let result = extract_variable_info(&tree, &node, source, &location);
                        
                        if let Some(expected_type) = expected_inferred_type {
                            assert!(result.is_some(), "Expected hover info for: {}", source);
                            let hover_text = result.unwrap();
                            assert!(hover_text.contains(&format!("*Inferred type: `{}`*", expected_type)),
                                   "Expected inferred type '{}' in hover text: {}", expected_type, hover_text);
                        } else {
                            // For explicit types, we should get hover info but without inferred type annotation
                            if let Some(hover_text) = result {
                                assert!(!hover_text.contains("*Inferred type:"), 
                                       "Should not show inferred type for explicit type annotation: {}", source);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_hover_numeric_literal_variations() {
        // Test specific Groovy numeric literal behavior
        let test_cases = vec![
            ("def a = 42", "Integer"),           // Plain integer
            ("def b = 42L", "Long"),            // Long literal  
            ("def c = 42l", "Long"),            // Long literal (lowercase)
            ("def d = 3.14", "BigDecimal"),     // Default decimal in Groovy
            ("def e = 3.14D", "Double"),        // Explicit Double
            ("def f = 3.14d", "Double"),        // Explicit Double (lowercase)
            ("def g = 3.14F", "Float"),         // Explicit Float
            ("def h = 3.14f", "Float"),         // Explicit Float (lowercase)
        ];

        for (source, expected_type) in test_cases {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let location = Location {
                        uri: "file:///test.groovy".parse().unwrap(),
                        range: Range {
                            start: Position { line: 0, character: 4 }, // Position at variable name
                            end: Position { line: 0, character: 5 },
                        },
                    };

                    let node = location_to_node(&location, &tree);
                    if let Some(node) = node {
                        let result = extract_variable_info(&tree, &node, source, &location);
                        
                        assert!(result.is_some(), "Expected hover info for: {}", source);
                        let hover_text = result.unwrap();
                        assert!(hover_text.contains(&format!("*Inferred type: `{}`*", expected_type)),
                               "Expected inferred type '{}' in hover text for '{}': {}", expected_type, source, hover_text);
                    }
                }
            }
        }
    }

    #[test]
    fn test_hover_shows_inferred_types_for_expected_cases() {
        // This test demonstrates the expected behavior with comment-style annotations
        let groovy_code_with_expected_types = vec![
            "def a = 42",                    // Integer
            "def b = \"hello\"",            // String
            "def c = 4.2",                   // BigDecimal
            "def d = 42L",                   // Long
            "def e = true",                  // Boolean
            "def f = [1, 2, 3]",            // List<Integer>
            "def g = [\"abc\"]",            // List<String>
            "def h = [a: 1, b: 2]",         // Map<String, Integer>
            "def i = new File(\"path\")",   // File
            "def j = new ArrayList<String>()", // List<String>
        ];

        // This test demonstrates that type inference works for various Groovy constructs
        for source in groovy_code_with_expected_types {
            if let Some(mut parser) = create_groovy_parser() {
                if let Some(tree) = parser.parse(source, None) {
                    let location = Location {
                        uri: "file:///test.groovy".parse().unwrap(),
                        range: Range {
                            start: Position { line: 0, character: 4 }, // Position at variable name
                            end: Position { line: 0, character: 5 },
                        },
                    };

                    let node = location_to_node(&location, &tree);
                    if let Some(node) = node {
                        let result = extract_variable_info(&tree, &node, source, &location);
                        // Verify that we get type inference for each case
                        assert!(result.is_some(), "Expected hover info for: {}", source);
                        let hover_text = result.unwrap();
                        assert!(hover_text.contains("*Inferred type:"), 
                               "Expected inferred type in hover for: {}", source);
                    }
                }
            }
        }
    }
}
