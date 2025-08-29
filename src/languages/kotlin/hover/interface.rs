use tree_sitter::{Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::common::hover::{format_inheritance, HoverSignature};

#[tracing::instrument(skip_all)]
pub fn extract_interface_signature(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"
    (package_header (identifier) @package_name)

    (interface_declaration
        (type_identifier) @interface_name
        (type_parameters)? @type_params
        (delegation_specifier)* @supertypes
    )

    (interface_declaration (modifiers (annotation) @annotation))

    (interface_declaration (modifiers (visibility_modifier) @modifier))

    (interface_declaration (modifiers (class_modifier) @modifier))

    (interface_declaration (modifiers (function_modifier) @modifier))

    (_
        (multiline_comment) @kdoc
        .
        (interface_declaration
            (modifiers)?
            (type_identifier) @interface_name
            (type_parameters)? @type_params
            (delegation_specifier)* @supertypes
        )
    )
    "#;

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut package_name = String::new();
    let mut interface_name = String::new();
    let mut type_params = String::new();
    let mut supertypes = String::new();
    let mut annotations = Vec::new();
    let mut modifiers = Vec::new();
    let mut kdoc = String::new();

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
                "annotation" => {
                    annotations.push(text.to_string());
                }
                "modifier" => {
                    modifiers.push(text.to_string());
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
                "kdoc" => {
                    if kdoc.is_empty() {
                        kdoc.push_str(text);
                    }
                }
                _ => {}
            }
        }
    }

    if interface_name.is_empty() {
        return None;
    }

    // Annotations and modifiers are already separated by the query

    // Build signature line
    let mut signature_line = String::new();
    signature_line.push_str("interface ");
    signature_line.push_str(&interface_name);

    if !type_params.is_empty() {
        signature_line.push_str(&type_params.replace('\n', " "));
    }

    let hover = HoverSignature::new("kotlin")
        .with_package(if package_name.is_empty() {
            None
        } else {
            Some(package_name)
        })
        .with_annotations(annotations)
        .with_modifiers(modifiers)
        .with_signature_line(signature_line)
        .with_inheritance(format_inheritance(&supertypes))
        .with_documentation(if kdoc.is_empty() { None } else { Some(kdoc) });

    Some(hover.format())
}
