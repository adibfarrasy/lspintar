use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{parse_parameters, format_parameters, HoverSignature};

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Find the function declaration node that contains this node
    let method_node = find_method_node(node)?;
    
    
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    
    (
      (block_comment)? @javadoc
      .
      (function_declaration
        (modifiers 
          (annotation)* @annotation
          (marker_annotation)* @annotation
          "public"? @modifier
          "private"? @modifier  
          "protected"? @modifier
          "static"? @modifier
          "final"? @modifier
          "abstract"? @modifier
          "synchronized"? @modifier
          "native"? @modifier
          "strictfp"? @modifier
        )?
        type: (_) @return_type
        name: (identifier) @method_name
        parameters: (parameters) @parameters
        (throws)? @throws_clause
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut method_name = String::new();
    let mut parameters = String::new();
    let mut return_type = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut throws_clause = String::new();
    let mut javadoc = String::new();
    
    // First get package name from tree root
    let mut pkg_matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = pkg_matches.next() {
        for capture in query_match.captures {
            if query.capture_names()[capture.index as usize] == "package_name" {
                if package_name.is_empty() {
                    package_name = capture.node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                }
            }
        }
    }
    
    // Reset cursor for method search - search from tree root to capture javadoc
    cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    // Find the specific method match
    while let Some(query_match) = matches.next() {
        let mut found_target_method = false;
        
        // First check if this match contains our target method
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            
            // Check if we found a function_declaration that encompasses our target node
            if capture_name == "method_name" {
                // Check if the method name node is within our target method_node
                if capture.node.start_byte() >= method_node.start_byte() && 
                   capture.node.end_byte() <= method_node.end_byte() {
                    found_target_method = true;
                    break;
                }
            }
        }
        
        // If this is our target method, extract all the data
        if found_target_method {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "annotation" => annotations.push(text.to_string()),
                    "modifier" => modifiers.push(text.to_string()),
                    "return_type" => return_type.push_str(text),
                    "method_name" => method_name = text.to_string(),
                    "parameters" => parameters.push_str(text),
                    "throws_clause" => throws_clause = text.to_string(),
                    "javadoc" => javadoc = text.to_string(),
                    _ => {}
                }
            }
            break;
        }
    }


    format_method_signature(
        package_name,
        annotations,
        modifiers,
        return_type,
        method_name,
        parameters,
        throws_clause,
        javadoc,
    )
}

#[tracing::instrument(skip_all)]
fn find_method_node<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "function_declaration" | "constructor_declaration" => return Some(current_node),
            _ => current = current_node.parent(),
        }
    }
    
    None
}

#[tracing::instrument(skip_all)]
fn format_method_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    return_type: String,
    method_name: String,
    parameters: String,
    throws_clause: String,
    javadoc: String,
) -> Option<String> {
    if method_name.is_empty() {
        return None;
    }

    // Parse parameters and format them according to ≤3 vs >3 rule
    let param_list = parse_parameters(&parameters);
    let formatted_params = format_parameters(&param_list);

    let mut signature_line = String::new();
    
    // Add modifiers first (public, static, etc.)
    if !modifiers.is_empty() {
        signature_line.push_str(&modifiers.join(" "));
        signature_line.push(' ');
    }
    
    if !return_type.is_empty() {
        signature_line.push_str(&return_type);
        signature_line.push(' ');
    }

    signature_line.push_str(&method_name);
    signature_line.push_str(&formatted_params);

    if !throws_clause.is_empty() {
        signature_line.push(' ');
        signature_line.push_str(&throws_clause);
    }

    let hover = HoverSignature::new("java")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(Vec::new()) // Don't double-add modifiers since we already put them in signature_line
        .with_signature_line(signature_line)
        .with_documentation(if javadoc.is_empty() { None } else { Some(javadoc) });

    Some(hover.format())
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
    fn test_find_method_node() {
        let source = r#"
class TestClass {
    public void testMethod(String param) {
        System.out.println("test");
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the method identifier
        let mut cursor = root.walk();
        let mut method_identifier = None;
        
        fn find_identifier_by_name<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, target: &str, source: &str) -> Option<tree_sitter::Node<'a>> {
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
                    if let Some(found) = find_identifier_by_name(cursor, target, source) {
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
        
        method_identifier = find_identifier_by_name(&mut cursor, "testMethod", source);
        
        if let Some(node) = method_identifier {
            let method_node = find_method_node(&node);
            assert!(method_node.is_some());
            let method_decl = method_node.unwrap();
            assert_eq!(method_decl.kind(), "function_declaration");
        }
    }

    #[test]
    fn test_extract_method_signature_simple() {
        let source = r#"
package com.example;

class TestClass {
    /**
     * A simple test method
     */
    public void testMethod(String name) {
        System.out.println(name);
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the method identifier
        let mut cursor = root.walk();
        let mut method_identifier = None;
        
        fn find_method_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
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
                    if let Some(found) = find_method_identifier(cursor, source) {
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
        
        method_identifier = find_method_identifier(&mut cursor, source);
        
        if let Some(node) = method_identifier {
            let result = extract_method_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("```java"));
            assert!(content.contains("public void testMethod(String name)"));
            assert!(content.contains("A simple test method"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_method_signature_with_annotations() {
        let source = r#"
package com.example;

class TestClass {
    @Override
    @Deprecated
    public String complexMethod(String param1, int param2, boolean param3) throws Exception {
        return param1;
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the method identifier
        let mut cursor = root.walk();
        let mut method_identifier = None;
        
        fn find_method_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "complexMethod" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_method_identifier(cursor, source) {
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
        
        method_identifier = find_method_identifier(&mut cursor, source);
        
        if let Some(node) = method_identifier {
            let result = extract_method_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("@Override"));
            assert!(content.contains("@Deprecated"));
            assert!(content.contains("public String complexMethod(String param1, int param2, boolean param3)"));
            assert!(content.contains("throws Exception"));
        }
    }

    #[test]
    fn test_extract_method_signature_many_parameters() {
        let source = r#"
package com.example;

class TestClass {
    public void manyParamMethod(String param1, int param2, boolean param3, double param4, float param5) {
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the method identifier
        let mut cursor = root.walk();
        let mut method_identifier = None;
        
        fn find_method_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "manyParamMethod" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_method_identifier(cursor, source) {
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
        
        method_identifier = find_method_identifier(&mut cursor, source);
        
        if let Some(node) = method_identifier {
            let result = extract_method_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("public void manyParamMethod"));
            // Should use multi-line format for >3 parameters
            assert!(content.contains("    String param1,"));
            assert!(content.contains("    int param2,"));
            assert!(content.contains("    boolean param3,"));
            assert!(content.contains("    double param4,"));
            assert!(content.contains("    float param5,"));
        }
    }

    #[test]
    fn test_format_method_signature_minimal() {
        let result = format_method_signature(
            String::new(),
            Vec::new(),
            Vec::new(),
            String::new(),
            "testMethod".to_string(),
            "()".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("testMethod()"));
        assert!(!content.contains("package"));
        assert!(!content.contains("throws"));
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_format_method_signature_complete() {
        let result = format_method_signature(
            "com.example.test".to_string(),
            vec!["@Override".to_string(), "@Deprecated".to_string()],
            vec!["public".to_string(), "static".to_string()],
            "String".to_string(),
            "complexMethod".to_string(),
            "(String param1, int param2)".to_string(),
            "throws Exception".to_string(),
            "/** Method documentation */".to_string()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("package com.example.test"));
        assert!(content.contains("@Override"));
        assert!(content.contains("@Deprecated"));
        assert!(content.contains("public static String complexMethod(String param1, int param2) throws Exception"));
        assert!(content.contains("---"));
        assert!(content.contains("Method documentation"));
    }
}