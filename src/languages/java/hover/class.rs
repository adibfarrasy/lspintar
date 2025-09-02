use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{deduplicate_modifiers, format_inheritance_items, HoverSignature};

/// Find the class declaration node that contains or corresponds to the given node
#[tracing::instrument(skip_all)]
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
        (modifiers) @modifiers
        name: (identifier) @class_name
        type_parameters: (type_parameters)? @type_parameters
        superclass: (superclass)? @superclass_line
        interfaces: (super_interfaces)? @interface_line
    )

    (_
        (block_comment) @javadoc
        .
        (class_declaration
            (modifiers) @modifiers
            name: (identifier) @class_name
            type_parameters: (type_parameters)? @type_parameters
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
    let mut type_parameters = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
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

        // Check if this match corresponds to our target class
        let mut target_class_name_node = None;
        
        // Find the class_name capture that matches our target
        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            if capture_name == "class_name" {
                // Check if this class_name node is within our target class declaration
                // The target_class_node from find_target_class_node is the class_declaration,
                // but we want to match the identifier (class_name) within it
                if capture.node.start_byte() >= target_class_node.start_byte() 
                   && capture.node.end_byte() <= target_class_node.end_byte() {
                    // Additional check: make sure this is the direct class_name of target_class_node
                    if let Some(class_name_field) = target_class_node.child_by_field_name("name") {
                        if capture.node.start_byte() == class_name_field.start_byte()
                           && capture.node.end_byte() == class_name_field.end_byte() {
                            target_class_name_node = Some(capture.node);
                            break;
                        }
                    }
                }
            }
        }
        
        // Only process class-specific captures if this match contains our target class
        if let Some(_target_node) = target_class_name_node {
            // Process all captures for this specific match
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => {
                        // Already handled above
                    }
                    "modifiers" => {
                        // Clear and populate modifiers for this specific match
                        modifiers.clear();
                        annotations.clear();
                        
                        // Parse all modifiers and annotations from the modifiers node
                        let mod_text = text;
                        for word in mod_text.split_whitespace() {
                            if word.starts_with('@') {
                                annotations.push(word.to_string());
                            } else if matches!(word, "public" | "private" | "protected" | "static" | "final" | "abstract" | "sealed" | "non-sealed" | "strictfp") {
                                modifiers.push(word.to_string());
                            }
                        }
                    }
                    "class_name" => {
                        if class_name.is_empty() {
                            class_name.push_str(text);
                        }
                    }
                    "type_parameters" => {
                        if type_parameters.is_empty() {
                            type_parameters = text.to_string();
                        }
                    }
                    "interface_line" => {
                        if interface_line.is_empty() {
                            interface_line = text.to_string();
                        }
                    }
                    "superclass_line" => {
                        if superclass_line.is_empty() {
                            superclass_line = text.to_string();
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
            // Break after processing the correct match - no need to continue
            break;
        }
    }

    format_class_signature(
        package_name,
        annotations,
        deduplicate_modifiers(modifiers),
        class_name,
        type_parameters,
        interface_line,
        superclass_line,
        javadoc,
    )
}

#[tracing::instrument(skip_all)]
fn format_class_signature(
    package_name: String,
    annotations: Vec<String>,
    modifiers: Vec<String>,
    class_name: String,
    type_parameters: String,
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
    if !type_parameters.is_empty() {
        signature_line.push_str(&type_parameters);
    }

    // Format inheritance with <= 3 rule
    let mut inheritance_parts = Vec::new();
    
    // Parse extends clause
    if !superclass_line.is_empty() {
        let extends_items: Vec<String> = superclass_line
            .strip_prefix("extends")
            .unwrap_or(&superclass_line)
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !extends_items.is_empty() {
            if let Some(formatted) = format_inheritance_items(&extends_items, "extends") {
                inheritance_parts.push(formatted);
            }
        }
    }
    
    // Parse implements clause
    if !interface_line.is_empty() {
        let implements_items: Vec<String> = interface_line
            .strip_prefix("implements")
            .unwrap_or(&interface_line)
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !implements_items.is_empty() {
            if let Some(formatted) = format_inheritance_items(&implements_items, "implements") {
                inheritance_parts.push(formatted);
            }
        }
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
            String::new(),
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
            String::new(),
            String::new()
        );
        
        assert!(result.is_none());
    }
    
    #[test]
    fn test_format_class_signature_no_duplicate_modifiers() {
        // Test that deduplicate_modifiers removes duplicates
        let modifiers_with_duplicates = vec!["public".to_string(), "final".to_string(), "public".to_string(), "final".to_string()];
        let deduplicated = deduplicate_modifiers(modifiers_with_duplicates);
        
        let result = format_class_signature(
            "com.example".to_string(),
            vec!["@Component".to_string()],
            deduplicated,
            "TestClass".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // Should only have one occurrence of each modifier
        assert_eq!(content.matches("public").count(), 1);
        assert_eq!(content.matches("final").count(), 1);
    }
    
    #[test]
    fn test_format_class_signature_three_or_less_interfaces() {
        let result = format_class_signature(
            "com.example".to_string(),
            Vec::new(),
            vec!["public".to_string()],
            "TestClass".to_string(),
            String::new(),
            "implements Serializable, Cloneable, Comparable".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // With 3 interfaces, should be inline
        assert!(content.contains("implements Serializable, Cloneable, Comparable"));
        // Should not have extra newlines for implements
        let lines: Vec<&str> = content.lines().collect();
        let implements_line = lines.iter().find(|&&l| l.contains("implements")).unwrap();
        assert!(implements_line.contains("Serializable") && implements_line.contains("Cloneable") && implements_line.contains("Comparable"));
    }
    
    #[test]
    fn test_format_class_signature_more_than_three_interfaces() {
        let result = format_class_signature(
            "com.example".to_string(),
            Vec::new(),
            vec!["public".to_string()],
            "TestClass".to_string(),
            String::new(),
            "implements Interface1, Interface2, Interface3, Interface4, Interface5".to_string(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // With more than 3 interfaces, should be multi-line
        assert!(content.contains("implements\n"));
        assert!(content.contains("    Interface1,"));
        assert!(content.contains("    Interface2,"));
        assert!(content.contains("    Interface3,"));
        assert!(content.contains("    Interface4,"));
        assert!(content.contains("    Interface5"));
    }
    
    #[test]
    fn test_format_class_signature_extends_and_implements() {
        let result = format_class_signature(
            "com.example".to_string(),
            Vec::new(),
            vec!["public".to_string()],
            "TestClass".to_string(),
            String::new(),
            "implements Interface1, Interface2".to_string(),
            "extends BaseClass".to_string(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        // Extends should be on one line (only 1 item)
        assert!(content.contains("extends BaseClass"));
        // Implements with 2 items should be inline
        assert!(content.contains("implements Interface1, Interface2"));
    }
    
    #[test]
    fn test_extract_class_signature_with_duplicate_modifiers() {
        let source = r#"
package com.example;

public final class TestClass {
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
                        if text == "TestClass" {
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
            println!("Content: {}", content);
            // Should only have one occurrence of each modifier
            assert_eq!(content.matches("public").count(), 1);
            assert_eq!(content.matches("final").count(), 1);
            // Should not contain incorrect modifiers like "private" or "static"
            assert_eq!(content.matches("private").count(), 0);
            assert_eq!(content.matches("static").count(), 0);
        }
    }
    
    #[test]
    fn test_format_class_signature_with_generics() {
        let result = format_class_signature(
            "com.example".to_string(),
            Vec::new(),
            vec!["public".to_string()],
            "GenericClass".to_string(),
            "<T, U extends Serializable>".to_string(),
            String::new(),
            String::new(),
            String::new()
        );
        
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("public class GenericClass<T, U extends Serializable>"));
    }
    
    #[test]
    fn test_extract_class_signature_like_java_lang_class() {
        let source = r#"
package java.lang;

public final class Class<T> 
implements java.io.Serializable, GenericDeclaration, Type, AnnotatedElement {
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
                        if text == "Class" {
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
            println!("Java.lang.Class content: {}", content);
            // Should only have correct modifiers
            assert_eq!(content.matches("public").count(), 1);
            assert_eq!(content.matches("final").count(), 1);
            // Should not contain incorrect modifiers
            assert_eq!(content.matches("private").count(), 0);
            assert_eq!(content.matches("static").count(), 0);
            // Should contain proper inheritance formatting (more than 3 interfaces)
            assert!(content.contains("implements\n"));
            assert!(content.contains("java.io.Serializable"));
            assert!(content.contains("GenericDeclaration"));
            assert!(content.contains("Type"));
            assert!(content.contains("AnnotatedElement"));
            // Should contain generics
            assert!(content.contains("class Class<T>"));
        }
    }
    
    #[test]
    fn test_extract_private_static_class() {
        let source = r#"
package com.example;

private static class InnerClass<T> extends BaseClass {
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
                        if text == "InnerClass" {
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
            println!("Private static class content: {}", content);
            // Should have exactly the right modifiers
            assert_eq!(content.matches("private").count(), 1);
            assert_eq!(content.matches("static").count(), 1);
            // Should not have incorrect modifiers
            assert_eq!(content.matches("public").count(), 0);
            assert_eq!(content.matches("final").count(), 0);
            // Should contain generics and extends
            assert!(content.contains("private static class InnerClass<T>"));
            assert!(content.contains("extends BaseClass"));
        }
    }
    
    #[test]
    fn test_extract_nested_classes_like_java_lang_class() {
        let source = r#"
package java.lang;

public final class Class<T> implements Serializable {
    private static class EnclosingMethodInfo {
    }
    
    private static class ReflectionData<T> {
    }
}
        "#;
        
        let tree = create_test_tree(source);
        let root = tree.root_node();
        
        // Find the outer Class identifier
        let mut cursor = root.walk();
        let mut outer_class_node = None;
        
        fn find_first_class_identifier<'a>(cursor: &mut tree_sitter::TreeCursor<'a>, source: &str) -> Option<tree_sitter::Node<'a>> {
            loop {
                let node = cursor.node();
                if node.kind() == "identifier" {
                    if let Ok(text) = node.utf8_text(source.as_bytes()) {
                        if text == "Class" {
                            return Some(node);
                        }
                    }
                }
                
                if cursor.goto_first_child() {
                    if let Some(found) = find_first_class_identifier(cursor, source) {
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
        
        outer_class_node = find_first_class_identifier(&mut cursor, source);
        
        if let Some(node) = outer_class_node {
            let result = extract_class_signature(&tree, &node, source);
            assert!(result.is_some());
            let content = result.unwrap();
            println!("Outer Class content: {}", content);
            // Should have exactly the right modifiers for the OUTER class
            assert_eq!(content.matches("public").count(), 1);
            assert_eq!(content.matches("final").count(), 1);
            // Should NOT have modifiers from nested classes
            assert_eq!(content.matches("private").count(), 0);
            assert_eq!(content.matches("static").count(), 0);
            // Should contain the outer class signature
            assert!(content.contains("public final class Class<T>"));
            assert!(content.contains("implements Serializable"));
        }
    }
}
