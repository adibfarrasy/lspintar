use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Find the field declaration node that contains this node
    let field_node = find_field_node(node)?;
    
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    
    (
      (block_comment)? @javadoc
      .
      (field_declaration
        (modifiers 
          [
            (annotation)
            (marker_annotation)
          ]* @annotation
          "public"? @modifier
          "private"? @modifier  
          "protected"? @modifier
          "static"? @modifier
          "final"? @modifier
          "transient"? @modifier
          "volatile"? @modifier
        )? 
        type: (_) @field_type
        declarator: (variable_declarator
          name: (identifier) @field_name
          value: (_)? @field_value
        )
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut field_name = String::new();
    let mut field_type = String::new();
    let mut field_value = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
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
    
    // Reset cursor for field search - search from tree root to capture javadoc
    cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    // Find the specific field match
    while let Some(query_match) = matches.next() {
        let mut found_target_field = false;
        
        // First check if this match contains our target field
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            
            // Check if we found a field_name that's within our target field_node
            if capture_name == "field_name" {
                // Check if the field name node is within our target field_node
                if capture.node.start_byte() >= field_node.start_byte() && 
                   capture.node.end_byte() <= field_node.end_byte() {
                    found_target_field = true;
                    break;
                }
            }
        }
        
        // If this is our target field, extract all the data
        if found_target_field {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "annotation" => annotations.push(text.to_string()),
                    "modifier" => modifiers.push(text.to_string()),
                    "field_type" => field_type.push_str(text),
                    "field_name" => field_name = text.to_string(),
                    "field_value" => field_value = text.to_string(),
                    "javadoc" => javadoc = text.to_string(),
                    _ => {}
                }
            }
            break;
        }
    }

    format_field_signature(
        package_name,
        annotations,
        modifiers,
        field_type,
        field_name,
        field_value,
        javadoc,
    )
}

fn find_field_node<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "field_declaration" => return Some(current_node),
            _ => current = current_node.parent(),
        }
    }
    
    None
}

fn format_field_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    field_type: String,
    field_name: String,
    field_value: String,
    javadoc: String,
) -> Option<String> {
    if field_name.is_empty() {
        return None;
    }

    let mut signature_line = String::new();
    
    if !field_type.is_empty() {
        signature_line.push_str(&field_type);
        signature_line.push(' ');
    }

    signature_line.push_str(&field_name);

    if !field_value.is_empty() {
        signature_line.push_str(" = ");
        signature_line.push_str(&field_value);
    }

    let hover = HoverSignature::new("java")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers)
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
    fn test_find_field_node() {
        let source = r#"
class TestClass {
    private String testField = "value";
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the field identifier
        let mut cursor = root.walk();
        let mut field_identifier = None;
        
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
        
        field_identifier = find_identifier_by_name(&mut cursor, "testField", source);
        
        if let Some(node) = field_identifier {
            let field_node = find_field_node(&node);
            assert!(field_node.is_some());
            let field_decl = field_node.unwrap();
            assert_eq!(field_decl.kind(), "field_declaration");
        }
    }

    #[test]
    fn test_extract_field_signature_simple() {
        let source = r#"
package com.example;

class TestClass {
    /**
     * A test field
     */
    private String testField;
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the field identifier
        let mut cursor = root.walk();
        let mut field_identifier = None;
        
        fn find_field_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "testField" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_field_identifier(cursor, source) {
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
        
        field_identifier = find_field_identifier(&mut cursor, source);
        
        if let Some(node) = field_identifier {
            let result = extract_field_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("```java"));
            assert!(content.contains("private String testField"));
            assert!(content.contains("A test field"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_field_signature_with_annotations() {
        let source = r#"
package com.example;

class TestClass {
    @Autowired
    @Qualifier("special")
    public static final String CONSTANT_FIELD = "constant_value";
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the field identifier
        let mut cursor = root.walk();
        let mut field_identifier = None;
        
        fn find_field_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "CONSTANT_FIELD" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_field_identifier(cursor, source) {
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
        
        field_identifier = find_field_identifier(&mut cursor, source);
        
        if let Some(node) = field_identifier {
            let result = extract_field_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("@Autowired"));
            assert!(content.contains("@Qualifier(\"special\")"));
            assert!(content.contains("public static final String CONSTANT_FIELD = \"constant_value\""));
        }
    }

    #[test]
    fn test_format_field_signature_minimal() {
        let result = format_field_signature(
            String::new(),
            Vec::new(),
            Vec::new(),
            "String".to_string(),
            "testField".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("String testField"));
        assert!(!content.contains("package"));
        assert!(!content.contains("="));
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_format_field_signature_complete() {
        let result = format_field_signature(
            "com.example.test".to_string(),
            vec!["@Autowired".to_string(), "@Qualifier(\"test\")".to_string()],
            vec!["public".to_string(), "static".to_string(), "final".to_string()],
            "String".to_string(),
            "CONSTANT".to_string(),
            "\"constant_value\"".to_string(),
            "/** Field documentation */".to_string()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("package com.example.test"));
        assert!(content.contains("@Autowired"));
        assert!(content.contains("@Qualifier(\"test\")"));
        assert!(content.contains("public static final String CONSTANT = \"constant_value\""));
        assert!(content.contains("---"));
        assert!(content.contains("Field documentation"));
    }

    #[test]
    fn test_format_field_signature_empty_name() {
        let result = format_field_signature(
            "com.example".to_string(),
            Vec::new(),
            Vec::new(),
            "String".to_string(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_none());
    }
}