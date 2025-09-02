use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{deduplicate_modifiers, format_inheritance_items, partition_modifiers, HoverSignature};

/// Find the interface declaration node that contains or corresponds to the given node
#[tracing::instrument(skip_all)]
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
        type_parameters: (type_parameters)? @type_parameters
        (extends_interfaces)? @extends_line
    )

    (_
        (block_comment) @javadoc
        .
        (interface_declaration
            (modifiers)? @modifiers
            name: (identifier) @interface_name
            type_parameters: (type_parameters)? @type_parameters
            (extends_interfaces)? @extends_line
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut extends_line = String::new();
    let mut type_parameters = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        // First, handle package_name (always process this from any match)
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "package_name" && package_name.is_empty() {
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                package_name.push_str(text);
            }
        }

        // Check if this match corresponds to our target interface
        let mut target_interface_name_node = None;
        
        // Find the interface_name capture that matches our target
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "interface_name" {
                // Check if this interface_name node is within our target interface declaration
                if capture.node.start_byte() >= target_interface_node.start_byte() 
                   && capture.node.end_byte() <= target_interface_node.end_byte() {
                    // Additional check: make sure this is the direct interface_name of target_interface_node
                    if let Some(interface_name_field) = target_interface_node.child_by_field_name("name") {
                        if capture.node.start_byte() == interface_name_field.start_byte()
                           && capture.node.end_byte() == interface_name_field.end_byte() {
                            target_interface_name_node = Some(capture.node);
                            break;
                        }
                    }
                }
            }
        }
        
        // Only process interface-specific captures if this match contains our target interface
        if let Some(_target_node) = target_interface_name_node {
            // Process all captures for this specific match
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => {
                        // Already handled above
                    },
                    "modifiers" => {
                        if modifiers.is_empty() {
                            modifiers.push_str(text);
                        }
                    },
                    "interface_name" => {
                        if interface_name.is_empty() {
                            interface_name.push_str(text);
                        }
                    },
                    "type_parameters" => {
                        if type_parameters.is_empty() {
                            type_parameters = text.to_string();
                        }
                    },
                    "extends_line" => {
                        if extends_line.is_empty() {
                            extends_line = text.to_string();
                        }
                    },
                    "javadoc" => {
                        if javadoc.is_empty() {
                            javadoc = text.to_string();
                        }
                    },
                    _ => {}
                }
            }
            // Break after processing the correct match - no need to continue
            break;
        }
    }
    

    format_interface_signature(
        package_name,
        modifiers,
        interface_name,
        type_parameters,
        extends_line,
        javadoc,
    )
}

#[tracing::instrument(skip_all)]
fn format_interface_signature(
    package_name: String,
    modifiers: String,
    interface_name: String,
    type_parameters: String,
    extends_line: String,
    javadoc: String,
) -> Option<String> {
    
    if interface_name.is_empty() {
        return None;
    }

    let (annotations, modifiers_vec) = partition_modifiers(&modifiers);
    let unique_modifiers = deduplicate_modifiers(modifiers_vec);

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("interface ");
    signature_line.push_str(&interface_name);
    if !type_parameters.is_empty() {
        signature_line.push_str(&type_parameters);
    }

    // Format extends with <= 3 rule
    let inheritance = if !extends_line.is_empty() {
        let extends_items: Vec<String> = extends_line
            .strip_prefix("extends")
            .unwrap_or(&extends_line)
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !extends_items.is_empty() {
            format_inheritance_items(&extends_items, "extends")
        } else {
            None
        }
    } else {
        None
    };

    let hover = HoverSignature::new("java")
        .with_package(if package_name.is_empty() { None } else { Some(package_name) })
        .with_annotations(annotations)
        .with_modifiers(unique_modifiers)
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
            String::new(),
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
            String::new(),
            String::new()
        );
        
        assert!(result.is_none());
    }
    
    #[test]
    fn test_format_interface_signature_three_or_less_extends() {
        let result = format_interface_signature(
            "com.example".to_string(),
            "public".to_string(),
            "TestInterface".to_string(),
            String::new(),
            "extends Interface1, Interface2, Interface3".to_string(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // With 3 interfaces, should be inline
        assert!(content.contains("extends Interface1, Interface2, Interface3"));
    }
    
    #[test]
    fn test_format_interface_signature_more_than_three_extends() {
        let result = format_interface_signature(
            "com.example".to_string(),
            "public".to_string(),
            "TestInterface".to_string(),
            String::new(),
            "extends Interface1, Interface2, Interface3, Interface4".to_string(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // With more than 3 interfaces, should be multi-line
        assert!(content.contains("extends\n"));
        assert!(content.contains("    Interface1,"));
        assert!(content.contains("    Interface2,"));
        assert!(content.contains("    Interface3,"));
        assert!(content.contains("    Interface4"));
    }
    
    #[test]
    fn test_format_interface_signature_no_duplicate_modifiers() {
        let result = format_interface_signature(
            "com.example".to_string(),
            "public static public static".to_string(),
            "TestInterface".to_string(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // Should only have one occurrence of each modifier (deduplicated)
        assert_eq!(content.matches("public").count(), 1);
        assert_eq!(content.matches("static").count(), 1);
    }
    
    #[test]
    fn test_format_interface_signature_with_generics() {
        let result = format_interface_signature(
            "com.example".to_string(),
            "public".to_string(),
            "GenericInterface".to_string(),
            "<T extends Serializable, U>".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("public interface GenericInterface<T extends Serializable, U>"));
    }
}