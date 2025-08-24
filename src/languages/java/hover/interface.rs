use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_interface_signature(tree: &Tree, source: &str) -> Option<String> {
    debug!("extract_interface_signature: Starting interface extraction");
    
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (
      (block_comment)? @javadoc
      (interface_declaration
        (modifiers)? @modifiers
        name: (identifier) @interface_name
        (extends_interfaces)? @extends_line
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse interface query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut extends_line = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    
    // Process all matches but avoid duplicate concatenation
    let mut found_interface = false;
    while let Some(query_match) = matches.next() {
        debug!("extract_interface_signature: Found interface match");
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
            debug!("extract_interface_signature: Captured '{}' = '{}'", capture_name, text);

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                },
                "modifiers" => {
                    if modifiers.is_empty() && !found_interface {
                        modifiers.push_str(text);
                    }
                },
                "interface_name" => {
                    if interface_name.is_empty() && !found_interface {
                        interface_name.push_str(text);
                        found_interface = true;
                    }
                },
                "extends_line" => {
                    if extends_line.is_empty() && !found_interface {
                        extends_line = text.to_string();
                    }
                },
                "javadoc" => {
                    if javadoc.is_empty() && !found_interface {
                        javadoc = text.to_string();
                    }
                },
                _ => {}
            }
        }
    }
    
    if !found_interface {
        debug!("extract_interface_signature: No interface matches found");
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
    debug!("format_interface_signature: interface_name='{}', modifiers='{}', package_name='{}', javadoc='{}'", 
           interface_name, modifiers, package_name, javadoc);
    
    if interface_name.is_empty() {
        debug!("format_interface_signature: interface_name is empty, returning None");
        return None;
    }

    let mut parts = Vec::new();

    if !package_name.is_empty() {
        parts.push(package_name);
        parts.push("\n".to_string());
    }

    parts.push("```java".to_string());

    let (annotation, modifier_vec) = partition_modifiers(modifiers);
    
    // Add annotations
    annotation.into_iter().for_each(|a| parts.push(a));

    // Build the interface declaration line
    let mut interface_line = String::new();
    
    if !modifier_vec.is_empty() {
        interface_line.push_str(&modifier_vec.join(" "));
        interface_line.push(' ');
    }
    
    interface_line.push_str("interface ");
    interface_line.push_str(&interface_name);
    
    parts.push(interface_line);

    // Add extends clause on separate line
    if !extends_line.is_empty() {
        parts.push(format!("    {}", extends_line));
    }

    parts.push("```".to_string());

    if !javadoc.is_empty() {
        parts.push("\n".to_string());
        parts.push("---".to_string());
        parts.push(javadoc);
    }

    Some(parts.join("\n"))
}