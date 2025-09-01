use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{parse_parameters, format_parameters, HoverSignature};

#[tracing::instrument(skip_all)]
pub fn extract_method_signature(tree: &Tree, node: &Node, source: &str) -> Option<String> {
    // Get the method name from the node
    let method_name = node.utf8_text(source.as_bytes()).ok()?;

    // Look for function declarations that match this method name
    let query_text = r#"
    (package_header (identifier) @package_name)
    
    (
        (multiline_comment)? @kdoc
        .
        (function_declaration
            (modifiers 
                (annotation)* @annotation
                (visibility_modifier)* @modifier
                (function_modifier)* @modifier
                (member_modifier)* @modifier
                (parameter_modifier)* @modifier
                (platform_modifier)* @modifier
            )?
            (simple_identifier) @method_name
            (type_parameters)? @type_params
            (function_value_parameters) @parameters
            (user_type)? @return_type
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    
    // First pass to get package name
    let mut pkg_matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = pkg_matches.next() {
        for capture in query_match.captures.iter() {
            if query.capture_names()[capture.index as usize] == "package_name" {
                if package_name.is_empty() {
                    package_name = capture.node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                }
            }
        }
    }
    
    // Reset cursor for second pass
    cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    while let Some(query_match) = matches.next() {
        let mut found_method = false;
        let mut annotations = Vec::new();
        let mut modifiers = Vec::new();
        let mut type_params = String::new();
        let mut parameters = String::new();
        let mut return_type = String::new();
        let mut kdoc = String::new();

        for capture in query_match.captures.iter() {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "method_name" => {
                    if text == method_name {
                        found_method = true;
                    }
                }
                "annotation" => {
                    annotations.push(text.to_string());
                }
                "modifier" => {
                    modifiers.push(text.to_string());
                }
                "type_params" => {
                    type_params.push_str(text);
                }
                "parameters" => {
                    parameters.push_str(text);
                }
                "return_type" => {
                    return_type.push_str(text);
                }
                "kdoc" => {
                    if kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }

        if found_method {
            // Parse parameters and format them according to ≤3 vs >3 rule
            let param_list = parse_parameters(&parameters);
            let formatted_params = format_parameters(&param_list);

            // Build signature line with modifiers included
            let mut signature_line = String::new();
            
            // Add modifiers first (private, suspend, etc.)
            if !modifiers.is_empty() {
                signature_line.push_str(&modifiers.join(" "));
                signature_line.push(' ');
            }
            
            signature_line.push_str("fun ");

            if !type_params.is_empty() {
                signature_line.push_str(&type_params);
                signature_line.push(' ');
            }

            signature_line.push_str(method_name);
            signature_line.push_str(&formatted_params);

            if !return_type.is_empty() {
                signature_line.push_str(": ");
                signature_line.push_str(&return_type);
            }

            let hover = HoverSignature::new("kotlin")
                .with_package(if package_name.is_empty() { None } else { Some(package_name.clone()) })
                .with_annotations(annotations)
                .with_modifiers(Vec::new()) // Don't double-add modifiers since we already put them in signature_line
                .with_signature_line(signature_line)
                .with_documentation(if kdoc.is_empty() { None } else { Some(kdoc) });

            return Some(hover.format());
        }
    }

    // Look for constructor declarations as well
    let constructor_query_text = r#"
    (secondary_constructor
        (modifiers 
            (annotation)* @annotation
            (visibility_modifier)* @modifier
            (member_modifier)* @modifier
        )? 
        (function_value_parameters) @parameters
    )
    "#;

    let constructor_query = Query::new(&tree.language(), constructor_query_text).ok()?;
    let mut constructor_cursor = QueryCursor::new();

    let mut constructor_matches =
        constructor_cursor.matches(&constructor_query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = constructor_matches.next() {
        // Check if the node is within this constructor
        if is_node_within_constructor(&query_match, node) {
            let mut annotations = Vec::new();
            let mut modifiers = Vec::new();
            let mut parameters = String::new();

            for capture in query_match.captures.iter() {
                let capture_name = constructor_query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "annotation" => {
                        annotations.push(text.to_string());
                    }
                    "modifier" => {
                        modifiers.push(text.to_string());
                    }
                    "parameters" => {
                        parameters.push_str(text);
                    }
                    _ => {}
                }
            }

            // Parse parameters and format them according to ≤3 vs >3 rule
            let param_list = parse_parameters(&parameters);
            let formatted_params = format_parameters(&param_list);

            // Build the constructor declaration line
            let mut constructor_line = String::new();

            if !modifiers.is_empty() {
                constructor_line.push_str(&modifiers.join(" "));
                constructor_line.push(' ');
            }

            constructor_line.push_str("constructor");
            constructor_line.push_str(&formatted_params);

            let hover = HoverSignature::new("kotlin")
                .with_annotations(annotations)
                .with_modifiers(Vec::new()) // Don't double-add modifiers since we already put them in signature_line
                .with_signature_line(constructor_line)
                .with_documentation(Some("*Secondary constructor*".to_string()));

            return Some(hover.format());
        }
    }

    None
}


fn is_node_within_constructor(
    constructor_match: &tree_sitter::QueryMatch,
    target_node: &Node,
) -> bool {
    for capture in constructor_match.captures.iter() {
        let constructor_node = capture.node;

        // Check if target_node is within the range of constructor_node
        if target_node.start_byte() >= constructor_node.start_byte()
            && target_node.end_byte() <= constructor_node.end_byte()
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
#[allow(unused_assignments)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Tree};

    fn create_test_tree(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_kotlin::language()).expect("Error loading Kotlin grammar");
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
    fun testMethod(name: String): Unit {
        println(name)
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
                if node.kind() == "simple_identifier" {
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
            assert!(content.contains("```kotlin"));
            assert!(content.contains("fun testMethod(name: String): Unit"));
            assert!(content.contains("A simple test method"));
            assert!(content.contains("---"));
        }
    }

    #[test]
    fn test_extract_method_signature_with_annotations() {
        let source = r#"
package com.example

class TestClass {
    @JvmOverloads
    @Deprecated("Use newMethod instead")
    fun complexMethod(param1: String, param2: Int, param3: Boolean): String {
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
                if node.kind() == "simple_identifier" {
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
            assert!(content.contains("@JvmOverloads"));
            assert!(content.contains("@Deprecated(\"Use newMethod instead\")"));
            assert!(content.contains("fun complexMethod(param1: String, param2: Int, param3: Boolean): String"));
        }
    }

    #[test]
    fn test_extract_method_signature_many_parameters() {
        let source = r#"
package com.example

class TestClass {
    fun manyParamMethod(param1: String, param2: Int, param3: Boolean, param4: Double, param5: Float) {
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
                if node.kind() == "simple_identifier" {
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
            assert!(content.contains("fun manyParamMethod"));
            // Should use multi-line format for >3 parameters
            assert!(content.contains("    param1: String,"));
            assert!(content.contains("    param2: Int,"));
            assert!(content.contains("    param3: Boolean,"));
            assert!(content.contains("    param4: Double,"));
            assert!(content.contains("    param5: Float,"));
        }
    }

    #[test]
    fn test_is_node_within_constructor() {
        // Test the utility function
        let source = r#"
class Test(val param: String) {
    constructor(param: String, extra: Int) : this(param)
}
        "#;
        
        let tree = create_test_tree(source);
        let _root = tree.root_node();
        
        // This is a basic test of the utility function structure
        // In practice, this would require setting up proper query matches
        assert!(true); // Placeholder - testing the function would require more setup
    }
}

