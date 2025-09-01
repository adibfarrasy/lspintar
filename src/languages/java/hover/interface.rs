use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::HoverSignature;

/// Find the interface declaration node that contains or corresponds to the given node
fn find_target_interface_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    
    while let Some(current_node) = current {
        match current_node.kind() {
            "interface_declaration" => return Some(current_node),
            "identifier" => {
                // If we're hovering over an identifier, check if its parent is an interface_declaration
                if let Some(parent) = current_node.parent() {
                    if parent.kind() == "interface_declaration" {
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
pub fn extract_interface_signature(tree: &Tree, node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Find the target interface node
    let target_interface_node = find_target_interface_node(node)?;
    
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (interface_declaration
        (modifiers)? @modifiers
        name: (identifier) @interface_name
        (extends_interfaces)? @extends_line
    )

    (_
        (block_comment) @javadoc
        .
        (interface_declaration
            (modifiers)? @modifiers
            name: (identifier) @interface_name
            (extends_interfaces)? @extends_line
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut extends_line = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        // Check if this match corresponds to our target interface
        let mut is_target_interface_match = false;
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "interface_name" {
                // Check if this interface_name capture is within our target interface node
                if capture.node.start_byte() >= target_interface_node.start_byte() 
                   && capture.node.end_byte() <= target_interface_node.end_byte() {
                    is_target_interface_match = true;
                    break;
                }
            }
        }

        // Only process captures for our target interface match
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                },
                "modifiers" => {
                    if is_target_interface_match && modifiers.is_empty() {
                        modifiers.push_str(text);
                    }
                },
                "interface_name" => {
                    if is_target_interface_match && interface_name.is_empty() {
                        interface_name.push_str(text);
                    }
                },
                "extends_line" => {
                    if is_target_interface_match && extends_line.is_empty() {
                        extends_line = text.to_string();
                    }
                },
                "javadoc" => {
                    if is_target_interface_match && javadoc.is_empty() {
                        javadoc = text.to_string();
                    }
                },
                _ => {}
            }
        }
    }
    

    format_interface_signature(
        package_name,
        modifiers,
        interface_name,
        extends_line,
        javadoc,
    )
}

fn format_interface_signature(
    package_name: String,
    modifiers: String,
    interface_name: String,
    extends_line: String,
    javadoc: String,
) -> Option<String> {
    
    if interface_name.is_empty() {
        return None;
    }

    use crate::languages::common::hover::partition_modifiers;
    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("interface ");
    signature_line.push_str(&interface_name);

    let hover = HoverSignature::new("java")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(modifiers_vec)
        .with_signature_line(signature_line)
        .with_inheritance(if extends_line.is_empty() { None } else { Some(extends_line) })
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
    fn test_find_target_interface_node() {
        let source = r#"
interface OuterInterface {
    interface InnerInterface {
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the InnerInterface identifier
        let mut cursor = root.walk();
        let mut inner_interface_identifier = None;
        
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
        
        inner_interface_identifier = find_identifier_by_name(&mut cursor, "InnerInterface", source);
        
        if let Some(node) = inner_interface_identifier {
            let target_interface = find_target_interface_node(&node);
            assert!(target_interface.is_some());
            let interface_node = target_interface.unwrap();
            assert_eq!(interface_node.kind(), "interface_declaration");
            
            // The target should be the InnerInterface, not OuterInterface
            if let Some(name_node) = interface_node.child_by_field_name("name") {
                if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                    assert_eq!(text, "InnerInterface");
                }
            }
        }
    }

    #[test]
    fn test_extract_interface_signature_simple() {
        let source = r#"
package com.example;

/**
 * A simple test interface
 */
public interface SimpleInterface {
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the interface identifier
        let mut cursor = root.walk();
        let mut interface_node = None;
        
        fn find_interface_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "SimpleInterface" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_interface_identifier(cursor, source) {
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
        
        interface_node = find_interface_identifier(&mut cursor, source);
        
        if let Some(node) = interface_node {
            let result = extract_interface_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("```java"));
            assert!(content.contains("public interface SimpleInterface"));
            assert!(content.contains("A simple test interface"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_interface_signature_with_extends() {
        let source = r#"
package com.example;

@FunctionalInterface
public interface ExtendedInterface extends Serializable, Cloneable {
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the interface identifier
        let mut cursor = root.walk();
        let mut interface_node = None;
        
        fn find_interface_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "ExtendedInterface" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_interface_identifier(cursor, source) {
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
        
        interface_node = find_interface_identifier(&mut cursor, source);
        
        if let Some(node) = interface_node {
            let result = extract_interface_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            assert!(content.contains("package com.example"));
            assert!(content.contains("@FunctionalInterface"));
            assert!(content.contains("public interface ExtendedInterface"));
            assert!(content.contains("extends Serializable, Cloneable"));
        }
    }

    #[test]
    fn test_format_interface_signature_minimal() {
        let result = format_interface_signature(
            String::new(),
            String::new(),
            "TestInterface".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("```java"));
        assert!(content.contains("interface TestInterface"));
        assert!(!content.contains("package"));
        assert!(!content.contains("extends"));
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_format_interface_signature_complete() {
        let result = format_interface_signature(
            "com.example.test".to_string(),
            "@FunctionalInterface public".to_string(),
            "ComplexInterface".to_string(),
            "extends Serializable".to_string(),
            "/** Interface documentation */".to_string()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("package com.example.test"));
        assert!(content.contains("@FunctionalInterface"));
        assert!(content.contains("public interface ComplexInterface"));
        assert!(content.contains("extends Serializable"));
        assert!(content.contains("---"));
        assert!(content.contains("Interface documentation"));
    }

    #[test]
    fn test_format_interface_signature_empty_name() {
        let result = format_interface_signature(
            "com.example".to_string(),
            "public".to_string(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_none());
    }
}