use log::debug;
use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

pub fn extract_class_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_declaration
      (scoped_identifier) @package_name)
    (class_declaration
      (modifiers)? @modifiers
      name: (identifier) @class_name
      interfaces: (super_interfaces)? @interface_line
      superclass: (superclass)? @superclass_line
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

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");

                match capture_name {
                    "package_name" => package_name.push_str(text),
                    "modifiers" => class_name.push_str(text),
                    "class_name" => {
                        if !class_name.is_empty() {
                            class_name.push_str(" ");
                        }
                        class_name.push_str("class");
                        class_name.push_str(text);
                    }
                    "interface_line" => interface_line = text.to_string(),
                    "superclass_line" => superclass_line = text.to_string(),
                    _ => {}
                }
            }
        });

    format_class_signature(package_name, class_name, interface_line, superclass_line)
}

fn format_class_signature(
    package_name: String,
    class_name: String,
    interface_line: String,
    superclass_line: String,
) -> Option<String> {
    if class_name.is_empty() {
        return None;
    }

    let mut parts = Vec::new();

    parts.push(package_name);
    parts.push("\n".to_string());

    parts.push("```groovy".to_string());
    parts.push(class_name);

    if !interface_line.is_empty() {
        parts.push(interface_line);
        parts.push("\n".to_string());
    }

    if !superclass_line.is_empty() {
        parts.push(superclass_line);
        parts.push("\n".to_string());
    }

    parts.push("```".to_string());
    parts.push("\n".to_string());

    parts.push("---".to_string());

    // TODO: Add docstring extraction
    parts.push("lorem ipsum".to_string());

    Some(parts.join("\n"))
}
