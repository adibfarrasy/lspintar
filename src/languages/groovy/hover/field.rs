use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

#[tracing::instrument(skip_all)]
pub fn extract_field_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (
      (block_comment)? @groovydoc
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
        type: (type_identifier) @field_type
        declarator: (variable_declarator
          name: (identifier) @field_name
          value: (_)? @initial_value
        ))
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut field_type = String::new();
    let mut initial_value = String::new();
    let mut found_field = false;
    let mut groovydoc = String::new();

    let node_text = node.utf8_text(source.as_bytes()).ok()?;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found_field {
                return;
            }

            let mut current_field_name = String::new();
            let mut temp_annotations = Vec::new();
            let mut temp_modifiers = Vec::new();
            let mut temp_field_type = String::new();
            let mut temp_initial_value = String::new();

            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name = text.to_string(),
                    "annotation" => temp_annotations.push(text.to_string()),
                    "marker_annotation" => temp_annotations.push(text.to_string()),
                    "modifier" => temp_modifiers.push(text.to_string()),
                    "field_type" => temp_field_type = text.to_string(),
                    "field_name" => current_field_name = text.to_string(),
                    "initial_value" => temp_initial_value = text.to_string(),
                    "groovydoc" => groovydoc = text.to_string(),
                    _ => {}
                }
            }

            if current_field_name == node_text {
                annotations = temp_annotations;
                modifiers = temp_modifiers;
                field_type = temp_field_type;
                initial_value = temp_initial_value;
                found_field = true;
            }
        });

    if !found_field {
        return None;
    }

    format_field_signature(
        package_name,
        annotations,
        modifiers,
        field_type,
        node_text.to_string(),
        initial_value,
        groovydoc,
    )
}

fn format_field_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    field_type: String,
    field_name: String,
    initial_value: String,
    groovydoc: String,
) -> Option<String> {

    let mut signature_line = String::new();
    
    if !field_type.is_empty() {
        signature_line.push_str(&field_type);
        signature_line.push(' ');
    } else {
        signature_line.push_str("def ");
    }

    signature_line.push_str(&field_name);

    if !initial_value.is_empty() {
        signature_line.push_str(" = ");
        signature_line.push_str(&initial_value);
    }

    let hover = HoverSignature::new("groovy")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers)
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
    fn test_extract_field_signature_simple() {
        let source = r#"
package com.example

class TestClass {
    /**
     * A test field
     */
    private String testField
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
            assert!(content.contains("```groovy"));
            assert!(content.contains("private String testField"));
            assert!(content.contains("A test field"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_field_signature_with_annotations() {
        let source = r#"
package com.example

class TestClass {
    @Autowired
    @Qualifier("special")
    public static final String CONSTANT_FIELD = "constant_value"
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
            String::new(),
            "testField".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```groovy"));
        assert!(content.contains("def testField"));
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
}
