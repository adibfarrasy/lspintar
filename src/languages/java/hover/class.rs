use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

/// Find the class declaration node that contains or corresponds to the given node
fn find_target_class_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "class_declaration" => return Some(current_node),
            "identifier" => {
                // If we're hovering over an identifier, check if its parent is a class_declaration
                if let Some(parent) = current_node.parent() {
                    if parent.kind() == "class_declaration" {
                        return Some(parent);
                    }
                }
                current = current_node.parent();
            }
            _ => current = current_node.parent(),
        }
    }
    
    None
}

#[tracing::instrument(skip_all)]
pub fn extract_class_signature(tree: &Tree, node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Find the target class node
    let target_class_node = find_target_class_node(node)?;
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (class_declaration
        (modifiers 
            (annotation)* @annotation
            (marker_annotation)* @annotation
            "public"? @modifier
            "private"? @modifier  
            "protected"? @modifier
            "static"? @modifier
            "final"? @modifier
            "abstract"? @modifier
        )?
        name: (identifier) @class_name
        superclass: (superclass)? @superclass_line
        interfaces: (super_interfaces)? @interface_line
    )

    (_
        (block_comment) @javadoc
        .
        (class_declaration
            (modifiers 
                (annotation)* @annotation
                (marker_annotation)* @annotation
                "public"? @modifier
                "private"? @modifier  
                "protected"? @modifier
                "static"? @modifier
                "final"? @modifier
                "abstract"? @modifier
            )?
            name: (identifier) @class_name
            superclass: (superclass)? @superclass_line
            interfaces: (super_interfaces)? @interface_line
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut interface_line = String::new();
    let mut superclass_line = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        // Check if this match corresponds to our target class
        let mut is_target_class_match = false;
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "class_name" {
                // Check if this class_name capture is within our target class node
                if capture.node.start_byte() >= target_class_node.start_byte() 
                   && capture.node.end_byte() <= target_class_node.end_byte() {
                    is_target_class_match = true;
                    break;
                }
            }
        }

        // Only process captures for our target class match
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                }
                "annotation" => {
                    if is_target_class_match {
                        annotations.push(text.to_string());
                    }
                }
                "modifier" => {
                    if is_target_class_match {
                        modifiers.push(text.to_string());
                    }
                }
                "class_name" => {
                    if is_target_class_match && class_name.is_empty() {
                        class_name.push_str(text);
                    }
                }
                "interface_line" => {
                    if is_target_class_match && interface_line.is_empty() {
                        interface_line = text.to_string();
                    }
                }
                "superclass_line" => {
                    if is_target_class_match && superclass_line.is_empty() {
                        superclass_line = text.to_string();
                    }
                }
                "javadoc" => {
                    if is_target_class_match && javadoc.is_empty() {
                        javadoc = text.to_string();
                    }
                }
                _ => {}
            }
        }
    }

    format_class_signature(
        package_name,
        annotations,
        modifiers,
        class_name,
        interface_line,
        superclass_line,
        javadoc,
    )
}

fn format_class_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    class_name: String,
    interface_line: String,
    superclass_line: String,
    javadoc: String,
) -> Option<String> {
    if class_name.is_empty() {
        return None;
    }

    // Annotations and modifiers are already separated

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("class ");
    signature_line.push_str(&class_name);

    // Separate inheritance clauses - extends and implements on separate lines
    let mut inheritance_parts = Vec::new();
    if !superclass_line.is_empty() {
        inheritance_parts.push(superclass_line);
    }
    if !interface_line.is_empty() {
        inheritance_parts.push(interface_line);
    }
    let inheritance = if inheritance_parts.is_empty() {
        None
    } else {
        Some(inheritance_parts.join("\n"))
    };

    let hover = HoverSignature::new("java")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers)
        .with_signature_line(signature_line)
        .with_inheritance(inheritance)
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
    fn test_find_target_class_node() {
        let source = r#"
class OuterClass {
    class InnerClass {
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the InnerClass identifier
        let mut cursor = root.walk();
        let mut inner_class_identifier = None;
        
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
        
        inner_class_identifier = find_identifier_by_name(&mut cursor, "InnerClass", source);
        
        if let Some(node) = inner_class_identifier {
            let target_class = find_target_class_node(&node);
            assert!(target_class.is_some());
            let class_node = target_class.unwrap();
            assert_eq!(class_node.kind(), "class_declaration");
            
            // The target should be the InnerClass, not OuterClass
            if let Some(name_node) = class_node.child_by_field_name("name") {
                if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                    assert_eq!(text, "InnerClass");
                }
            }
        }
    }

    #[test]
    fn test_extract_class_signature_simple() {
        let source = r#"
package com.example;

/**
 * A simple test class
 */
public class SimpleClass {
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the class identifier
        let mut cursor = root.walk();
        let mut class_node = None;
        
        fn find_class_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "SimpleClass" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_class_identifier(cursor, source) {
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
        
        class_node = find_class_identifier(&mut cursor, source);
        
        if let Some(node) = class_node {
            let result = extract_class_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("```java"));
            assert!(content.contains("public class SimpleClass"));
            assert!(content.contains("A simple test class"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_class_signature_with_inheritance() {
        let source = r#"
package com.example;

@Component
@Service  
public abstract class BaseService extends AbstractService implements Serializable, Cloneable {
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the class identifier
        let mut cursor = root.walk();
        let mut class_node = None;
        
        fn find_class_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "BaseService" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_class_identifier(cursor, source) {
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
        
        class_node = find_class_identifier(&mut cursor, source);
        
        if let Some(node) = class_node {
            let result = extract_class_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("@Component"));
            assert!(content.contains("@Service"));
            assert!(content.contains("public abstract class BaseService"));
            assert!(content.contains("extends AbstractService"));
            assert!(content.contains("implements Serializable, Cloneable"));
        }
    }

    #[test]
    fn test_format_class_signature_minimal() {
        let result = format_class_signature(
            String::new(),
            Vec::new(),
            Vec::new(),
            "TestClass".to_string(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("class TestClass"));
        assert!(!content.contains("package"));
        assert!(!content.contains("extends"));
        assert!(!content.contains("implements"));
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_format_class_signature_complete() {
        let result = format_class_signature(
            "com.example.test".to_string(),
            vec!["@Component".to_string(), "@Service".to_string()],
            vec!["public".to_string(), "abstract".to_string()],
            "BaseClass".to_string(),
            "implements Serializable".to_string(),
            "extends Object".to_string(),
            "/** Class documentation */".to_string()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("package com.example.test"));
        assert!(content.contains("@Component"));
        assert!(content.contains("@Service"));
        assert!(content.contains("public abstract class BaseClass"));
        assert!(content.contains("extends Object"));
        assert!(content.contains("implements Serializable"));
        assert!(content.contains("---"));
        assert!(content.contains("Class documentation"));
    }

    #[test]
    fn test_format_class_signature_empty_name() {
        let result = format_class_signature(
            "com.example".to_string(),
            Vec::new(),
            Vec::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_none());
    }
}
