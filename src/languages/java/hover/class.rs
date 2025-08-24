use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_class_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)

    (
      (block_comment)? @javadoc
      (class_declaration
        (modifiers)? @modifiers
        name: (identifier) @class_name
        superclass: (superclass)? @superclass_line
        interfaces: (super_interfaces)? @interface_line
      )
    )
    "#;

    let query = Query::new(&tree.language(), query_text)
        .inspect_err(|error| debug!("Failed to parse query: {error}"))
        .ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut class_name = String::new();
    let mut interface_line = String::new();
    let mut superclass_line = String::new();
    let mut modifiers = String::new();
    let mut javadoc = String::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

            match capture_name {
                "package_name" => {
                    if package_name.is_empty() {
                        package_name.push_str(text);
                    }
                }
                "modifiers" => {
                    if modifiers.is_empty() {
                        modifiers.push_str(text);
                    }
                }
                "class_name" => {
                    if class_name.is_empty() {
                        class_name.push_str(text);
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
                _ => {
                    debug!(
                        "extract_class_signature: Unknown capture '{}': '{}'",
                        capture_name, text
                    );
                }
            }
        }
    }

    debug!("extract_class_signature: Final values - package='{}', class='{}', modifiers='{}', superclass='{}', interfaces='{}'", 
           package_name, class_name, modifiers, superclass_line, interface_line);

    format_class_signature(
        package_name,
        modifiers,
        class_name,
        interface_line,
        superclass_line,
        javadoc,
    )
}

fn format_class_signature(
    package_name: String,
    modifiers: String,
    class_name: String,
    interface_line: String,
    superclass_line: String,
    javadoc: String,
) -> Option<String> {
    if class_name.is_empty() {
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

    // Build the class declaration line
    let mut class_line = String::new();

    if !modifier_vec.is_empty() {
        class_line.push_str(&modifier_vec.join(" "));
        class_line.push(' ');
    }

    class_line.push_str("class ");
    class_line.push_str(&class_name);

    parts.push(class_line);

    // Add extends clause on separate line
    if !superclass_line.is_empty() {
        parts.push(format!("{}", superclass_line));
    }

    // Add implements clause on separate line
    if !interface_line.is_empty() {
        parts.push(format!("{}", interface_line));
    }

    parts.push("```".to_string());

    if !javadoc.is_empty() {
        parts.push("\n".to_string());
        parts.push("---".to_string());
        parts.push(javadoc);
    }

    Some(parts.join("\n"))
}

