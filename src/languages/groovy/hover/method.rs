use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{parse_parameters, format_parameters, HoverSignature};

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (
      (groovydoc_comment)? @groovydoc
      .
      (method_declaration
        (modifiers 
          (annotation)* @annotation
          (marker_annotation)* @marker_annotation
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
        type: (_)? @return_type
        name: (identifier) @method_name
        parameters: (formal_parameters) @parameters
        (throws)? @throws_clause
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut return_type = String::new();
    let mut parameters = String::new();
    let mut throws_clause = String::new();
    let mut found_method = false;
    let mut groovydoc = String::new();

    let node_text = node.utf8_text(source.as_bytes()).ok()?;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found_method {
                return;
            }

            let mut current_method_name = String::new();
            let mut temp_annotations = Vec::new();
            let mut temp_modifiers = Vec::new();
            let mut temp_return_type = String::new();
            let mut temp_parameters = String::new();
            let mut temp_throws = String::new();

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name = text.to_string(),
                    "annotation" => temp_annotations.push(text.to_string()),
                    "marker_annotation" => temp_annotations.push(text.to_string()),
                    "modifier" => temp_modifiers.push(text.to_string()),
                    "return_type" => temp_return_type = text.to_string(),
                    "method_name" => current_method_name = text.to_string(),
                    "parameters" => temp_parameters = text.to_string(),
                    "throws_clause" => temp_throws = text.to_string(),
                    "groovydoc" => groovydoc = text.to_string(),
                    _ => {}
                }
            }

            if current_method_name == node_text {
                annotations = temp_annotations;
                modifiers = temp_modifiers;
                return_type = temp_return_type;
                parameters = temp_parameters;
                throws_clause = temp_throws;
                found_method = true;
            }
        });

    if !found_method {
        return None;
    }

    format_method_signature(
        package_name,
        annotations,
        modifiers,
        return_type,
        node_text.to_string(),
        parameters,
        throws_clause,
        groovydoc,
    )
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
    groovydoc: String,
) -> Option<String> {
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
    } else {
        signature_line.push_str("def ");
    }

    signature_line.push_str(&method_name);
    signature_line.push_str(&formatted_params);

    if !throws_clause.is_empty() {
        signature_line.push(' ');
        signature_line.push_str(&throws_clause);
    }

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(Vec::new()) // Don't double-add modifiers since we already put them in signature_line
        .with_signature_line(signature_line)
        .with_documentation(if groovydoc.is_empty() { None } else { Some(groovydoc) });

    Some(hover.format())
}

#[cfg(test)]
#[allow(unused_assignments)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Tree};

    fn create_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_groovy::language()).expect("Error loading Groovy grammar");
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_method_signature_simple() {
        let source = r#"
package com.example

class TestClass {
    /**
     * A simple test method
     */
    void testMethod(String name) {
        println name
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
            assert!(content.contains("```groovy"));
            assert!(content.contains("void testMethod(String name)"));
            assert!(content.contains("A simple test method"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_method_signature_with_annotations() {
        let source = r#"
package com.example

class TestClass {
    @Override
    @Deprecated
    String complexMethod(String param1, int param2, boolean param3) throws Exception {
        return param1
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
            assert!(content.contains("String complexMethod(String param1, int param2, boolean param3)"));
            assert!(content.contains("throws Exception"));
        }
    }

    #[test]
    fn test_extract_method_signature_many_parameters() {
        let source = r#"
package com.example

class TestClass {
    void manyParamMethod(String param1, int param2, boolean param3, double param4, float param5) {
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
            assert!(content.contains("void manyParamMethod"));
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
        assert!(content.contains("```groovy"));
        assert!(content.contains("def testMethod()"));
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
