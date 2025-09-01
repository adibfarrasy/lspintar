use class::extract_class_signature;
use field::extract_field_signature;
use interface::extract_interface_signature;
use method::extract_method_signature;
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{symbols::SymbolType, utils::location_to_node},
    languages::LanguageSupport,
};

mod class;
mod field;
mod interface;
mod method;
mod utils;

pub fn handle(
    tree: &Tree,
    source: &str,
    location: Location,
    language_support: &dyn LanguageSupport,
) -> Option<Hover> {
    let node = location_to_node(&location, tree)?;

    let symbol_type = language_support.determine_symbol_type_from_context(tree, &node, source).ok()?;

    let content = match symbol_type {
        SymbolType::ClassDeclaration => extract_class_signature(tree, &node, source),
        SymbolType::InterfaceDeclaration => extract_interface_signature(tree, &node, source),
        SymbolType::MethodDeclaration => extract_method_signature(tree, &node, source),
        SymbolType::FieldDeclaration => extract_field_signature(tree, &node, source),
        SymbolType::Type => {
            // Type could be class, interface, enum, etc. - need to check the actual node
            match node.kind() {
                "class_declaration" => extract_class_signature(tree, &node, source),
                "interface_declaration" => extract_interface_signature(tree, &node, source),
                "enum_declaration" => extract_enum_signature(tree, source),
                _ => {
                    // Try interface extraction first, then fall back to generic type info
                    extract_interface_signature(tree, &node, source)
                        .or_else(|| extract_class_signature(tree, &node, source))
                        .or_else(|| extract_type_usage_info(&node, source))
                }
            }
        }
        SymbolType::MethodCall => {
            // For method calls, try to find the declaration first, then extract signature
            if let Some(method_decl_node) = find_method_declaration_for_call(tree, &node, source) {
                extract_method_signature(tree, &method_decl_node, source)
            } else {
                // Fallback: provide basic method call info
                extract_method_call_info(&node, source)
            }
        }
        SymbolType::VariableDeclaration | SymbolType::VariableUsage => {
            extract_variable_info(tree, &node, source)
        }
        _ => None,
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

fn extract_enum_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (
      (block_comment)? @javadoc
      (enum_declaration
        (modifiers)? @modifiers
        name: (identifier) @enum_name
        interfaces: (super_interfaces)? @interface_line
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut enum_name = String::new();
    let mut interface_line = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    // Process all matches but avoid duplicate concatenation
    let mut found_enum = false;
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                }
                "modifiers" => {
                    if modifiers.is_empty() && !found_enum {
                        modifiers.push_str(text);
                    }
                }
                "enum_name" => {
                    if enum_name.is_empty() && !found_enum {
                        enum_name.push_str(text);
                        found_enum = true;
                    }
                }
                "interface_line" => {
                    if interface_line.is_empty() {
                        interface_line = text.to_string();
                    }
                }
                "javadoc" => {
                    if javadoc.is_empty() {
                        javadoc = text.to_string();
                    }
                }
                _ => {}
            }
        }
    }

    format_enum_signature(package_name, modifiers, enum_name, interface_line, javadoc)
}

fn format_enum_signature(
    package_name: String,
    modifiers: String,
    enum_name: String,
    interface_line: String,
    javadoc: String,
) -> Option<String> {
    if enum_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    if !package_name.is_empty() {
        parts.push(format!("package {}", package_name));
        parts.push("".to_string()); // Empty line after package
    }

    parts.push("```java".to_string());

    let mut enum_line = String::new();

    if !modifiers.is_empty() {
        enum_line.push_str(&modifiers);
        enum_line.push(' ');
    }

    enum_line.push_str("enum ");
    enum_line.push_str(&enum_name);

    // Add implements clause on same line
    if !interface_line.is_empty() {
        enum_line.push(' ');
        enum_line.push_str(&interface_line);
    }

    parts.push(enum_line);

    parts.push("```".to_string());

    if !javadoc.is_empty() {
        parts.push("\n".to_string());
        parts.push("---".to_string());
        parts.push(javadoc);
    }

    Some(parts.join("\n"))
}

fn extract_type_usage_info(node: &Node, source: &str) -> Option<String> {
    if let Ok(type_text) = node.utf8_text(source.as_bytes()) {
        Some(format!("```java\n{}\n```\n\n*Type reference*", type_text))
    } else {
        None
    }
}

fn extract_variable_info(_tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Try to find variable declaration (local variables, fields, parameters)
    let var_node = find_parent_of_kind(node, "variable_declaration")
        .or_else(|| find_parent_of_kind(node, "local_variable_declaration"))
        .or_else(|| find_parent_of_kind(node, "field_declaration"));

    if let Some(var_node) = var_node {
        if let Ok(var_text) = var_node.utf8_text(source.as_bytes()) {
            return Some(format!("```java\n{}\n```", var_text.trim()));
        }
    }

    None
}

/// Find method declaration for a method call within the same file
fn find_method_declaration_for_call<'a>(
    tree: &'a Tree,
    node: &Node,
    source: &str,
) -> Option<Node<'a>> {
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    let query_text = r#"
        (method_declaration
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

/// Provide basic method call information when declaration can't be found
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
                format!(
                    "```java\n{}.{}{}\n```\n\n*Method call - definition not found in current file*",
                    object_text, method_name, args_text
                )
            } else {
                format!(
                    "```java\n{}{}\n```\n\n*Method call - definition not found in current file*",
                    method_name, args_text
                )
            };

            return Some(call_info);
        }
        current = parent.parent();
    }

    // Fallback for standalone method name
    Some(format!(
        "```java\n{}\n```\n\n*Method reference*",
        method_name
    ))
}

fn find_parent_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(*node);
    }

    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return Some(parent);
        }
        current = parent.parent();
    }

    None
}

#[cfg(test)]
#[allow(unused_assignments)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Tree};

    fn create_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_java::LANGUAGE.into()).expect("Error loading Java grammar");
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_enum_signature() {
        let source = r#"
package com.example;

/**
 * Test enum documentation
 */
public enum Status implements Serializable {
    ACTIVE, INACTIVE
}
        "#;
        
        let tree = create_test_tree(source);
        let result = extract_enum_signature(&tree, source);
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("package com.example"));
        assert!(content.contains("```java"));
        assert!(content.contains("public enum Status"));
        assert!(content.contains("implements Serializable"));
        assert!(content.contains("Test enum documentation"));
    }

    #[test] 
    fn test_format_enum_signature_minimal() {
        let result = format_enum_signature(
            String::new(),
            String::new(), 
            "Color".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("enum Color"));
        assert!(!content.contains("package"));
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_format_enum_signature_complete() {
        let result = format_enum_signature(
            "com.example".to_string(),
            "public".to_string(),
            "Status".to_string(), 
            "implements Serializable".to_string(),
            "/** Enum documentation */".to_string()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("com.example"));
        assert!(content.contains("public enum Status"));
        assert!(content.contains("implements Serializable"));
        assert!(content.contains("---"));
        assert!(content.contains("Enum documentation"));
    }

    #[test]
    fn test_extract_type_usage_info() {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_java::LANGUAGE.into()).expect("Error loading Java grammar");
        
        let source = "String name;";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let type_node = root.descendant_for_byte_range(0, 6).unwrap(); // "String" node
        
        let result = extract_type_usage_info(&type_node, source);
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("String"));
        assert!(content.contains("Type reference"));
    }

    #[test]
    fn test_extract_variable_info() {
        let source = r#"
class Test {
    private int count = 0;
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the variable identifier node
        let mut cursor = root.walk();
        let mut variable_node = None;
        
        fn find_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, target: &str, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == target {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_identifier(cursor, target, source) {
                        return Some(found);
                    }
                    cursor.goto_parent();
                }
                
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            None
        }
        
        variable_node = find_identifier(&mut cursor, "count", source);
        
        if let Some(node) = variable_node {
            let result = extract_variable_info(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("```java"));
            assert!(content.contains("private int count = 0"));
        }
    }

    #[test]
    fn test_find_method_declaration_for_call() {
        let source = r#"
class Test {
    public void testMethod() {
        System.out.println("test");
    }
    
    public void caller() {
        testMethod();
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find method call node
        let mut cursor = root.walk();
        let mut call_node = None;
        
        fn find_method_call<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "method_invocation" {
                    if let Some(name_node) = node.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                            if text == "testMethod" {
                                return Some(name_node);
                            }
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_method_call(cursor, source) {
                        return Some(found);
                    }
                    cursor.goto_parent();
                }
                
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            None
        }
        
        call_node = find_method_call(&mut cursor, source);
        
        if let Some(node) = call_node {
            let result = find_method_declaration_for_call(&tree, &node, source);
            assert!(result.is_some());
            let decl_node = result.unwrap();
            assert_eq!(decl_node.kind(), "method_declaration");
        }
    }

    #[test]
    fn test_extract_method_call_info() {
        let source = "testMethod(arg1, arg2)";
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_java::LANGUAGE.into()).expect("Error loading Java grammar");
        
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        
        // Find the method name node
        let mut cursor = root.walk();
        let mut method_name_node = None;
        
        fn find_method_name<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "testMethod" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_method_name(cursor, source) {
                        return Some(found);
                    }
                    cursor.goto_parent();
                }
                
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            None
        }
        
        method_name_node = find_method_name(&mut cursor, source);
        
        if let Some(node) = method_name_node {
            let result = extract_method_call_info(&node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("```java"));
            assert!(content.contains("testMethod(arg1, arg2)"));
            assert!(content.contains("Method call - definition not found"));
        }
    }

    #[test]
    fn test_find_parent_of_kind() {
        let source = r#"
class Test {
    private int field = 42;
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find an identifier node
        let mut cursor = root.walk();
        let mut identifier_node = None;
        
        fn find_identifier_node<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, target: &str, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == target {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_identifier_node(cursor, target, source) {
                        return Some(found);
                    }
                    cursor.goto_parent();
                }
                
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            None
        }
        
        identifier_node = find_identifier_node(&mut cursor, "field", source);
        
        if let Some(node) = identifier_node {
            let class_node = find_parent_of_kind(&node, "class_declaration");
            assert!(class_node.is_some());
            assert_eq!(class_node.unwrap().kind(), "class_declaration");
            
            let field_node = find_parent_of_kind(&node, "field_declaration");
            assert!(field_node.is_some());
            assert_eq!(field_node.unwrap().kind(), "field_declaration");
            
            let nonexistent = find_parent_of_kind(&node, "interface_declaration");
            assert!(nonexistent.is_none());
        }
    }
}

