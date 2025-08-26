use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use super::utils::partition_modifiers;

#[tracing::instrument(skip_all)]
pub fn extract_interface_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)

    (class_declaration
        (modifiers)? @modifiers
        name: (type_identifier) @interface_name
        (type_parameters)? @type_params
        supertype_list: (delegation_specifiers)? @supertypes
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut type_params = String::new();
    let mut supertypes = String::new();
    let mut modifiers = String::new();

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
                "interface_name" => {
                    if interface_name.is_empty() {
                        interface_name.push_str(text);
                    }
                }
                "type_params" => {
                    if type_params.is_empty() {
                        type_params.push_str(text);
                    }
                }
                "supertypes" => {
                    if supertypes.is_empty() {
                        supertypes.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if interface_name.is_empty() {
        return None;
    }

    let (access_modifiers, other_modifiers) = partition_modifiers(&modifiers);

    let mut signature = String::new();
    signature.push_str("```kotlin\n");

    if !access_modifiers.is_empty() {
        signature.push_str(&access_modifiers);
        signature.push(' ');
    }

    if !other_modifiers.is_empty() {
        signature.push_str(&other_modifiers);
        signature.push(' ');
    }

    signature.push_str("interface ");
    signature.push_str(&interface_name);

    if !type_params.is_empty() {
        signature.push_str(&type_params);
    }

    if !supertypes.is_empty() {
        signature.push_str(" : ");
        signature.push_str(&supertypes.replace('\n', " "));
    }

    signature.push_str("\n```");

    if !package_name.is_empty() {
        signature.push_str(&format!("\n\n**Package:** `{}`", package_name));
    }

    Some(signature)
}