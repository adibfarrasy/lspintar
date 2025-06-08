use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        dependency_cache::DependencyCache,
        utils::{
            create_parser_for_language, detect_language_from_path, find_project_root,
            path_to_file_uri, uri_to_path,
        },
    },
    languages::groovy::symbols::SymbolType,
};

pub fn find_identifier_at_position<'a>(
    tree: &'a Tree,
    source: &str,
    position: Position,
) -> Result<Node<'a>> {
    let query_text = r#"
    (identifier) @identifier
    (type_identifier) @identifier
    "#;
    let query = Query::new(&tree.language(), query_text).context(format!(
        "[find_identifier_at_position] failed to create a new query"
    ))?;

    let mut result: Result<Node> = Err(anyhow!(format!(
        "[find_identifier_at_position] invalid data. position: {:#?}",
        position
    )));
    let mut found = false;

    let mut cursor = QueryCursor::new();
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|match_| {
            if found {
                return;
            };

            for capture in match_.captures.iter() {
                let node = capture.node;
                if node_contains_position(&node, position) {
                    result = Ok(node);
                    found = true;
                    return;
                }
            }
        });

    result
}

fn node_contains_position(node: &Node, position: Position) -> bool {
    let start = node.start_position();
    let end = node.end_position();

    let pos_line = position.line as usize;
    let pos_char = position.character as usize;

    (start.row < pos_line || (start.row == pos_line && start.column <= pos_char))
        && (pos_line < end.row || (pos_line == end.row && pos_char <= end.column))
}

fn determine_symbol_type_from_context(
    tree: &Tree,
    node: &Node,
    source: &str,
) -> Result<SymbolType> {
    let node_text = node.utf8_text(source.as_bytes())?;

    if let Some(manual_type) = check_complex_structures(node) {
        return Ok(manual_type);
    }

    let query_text = r#"
        ; DECLARATIONS
        ; Variable declarations
        (variable_declaration
          declarator: (variable_declarator
            name: (identifier) @variable_name))

        ; Field declarations  
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_name))

        ; Class declarations
        (class_declaration
          name: (identifier) @class_name)

        ; Interface declarations
        (interface_declaration
          name: (identifier) @interface_name)

        ; Method declarations
        (method_declaration
          name: (identifier) @method_name)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_name)

        ; Parameters
        (formal_parameter
          name: (identifier) @param_name)

        ; USAGES
        (field_access field: (identifier) @field_usage)
        (method_invocation name: (identifier) @method_usage)
        (argument_list (identifier) @arg_usage)
        (assignment_expression left: (identifier) @var_usage)
        (assignment_expression right: (identifier) @var_usage)

        ; Type identifiers
        (type_identifier) @type_name
    "#;

    let query = Query::new(&tree.language(), query_text)
        .context("[determine_symbol_type_from_context] failed to create query")?;
    let mut cursor = QueryCursor::new();

    let mut found = false;

    let mut result = Err(anyhow!("[determine_symbol_type_from_context] invalid data"));

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if found {
                return;
            }

            for capture in query_match.captures {
                let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap();
                if capture_text == node_text {
                    let capture_name = query.capture_names()[capture.index as usize];
                    let symbol = match capture_name {
                        "variable_name" => SymbolType::Variable,
                        "field_name" => SymbolType::Field,
                        "class_name" => SymbolType::Class,
                        "interface_name" => SymbolType::Interface,
                        "method_name" => SymbolType::Method,
                        "enum_name" => SymbolType::Enum,
                        "param_name" => SymbolType::Parameter,
                        "method_usage" => SymbolType::Function,
                        "type_name" => SymbolType::Type,
                        "field_usage" => SymbolType::Field,
                        _ => SymbolType::Variable,
                    };

                    result = Ok(symbol);
                    found = true;
                }
            }
        });

    result
}

fn check_complex_structures(node: &Node) -> Option<SymbolType> {
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "package_declaration" => {
                return Some(SymbolType::Package);
            }
            "scoped_identifier" => {
                if is_inside_package_declaration(&parent) {
                    return Some(SymbolType::Package);
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    None
}

fn is_inside_package_declaration(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "package_declaration" {
            return true;
        }
        current = parent.parent();
    }
    false
}

pub fn find_definition_location(
    tree: &Tree,
    source: &str,
    dependency_cache: Arc<DependencyCache>,
    file_uri: &str,
    usage_node: &Node,
) -> Result<Location> {
    // First search locally in current file
    if let Some(local_location) =
        search_local_definitions_for_location(tree, source, file_uri, usage_node)
    {
        return Ok(local_location);
    }

    // TODO: implement and test
    // Check if it's a builtin type
    // if let Some(builtin_location) = search_builtin_types(symbol, &dependency_cache) {
    //     return Some(builtin_location);
    // }

    // Search in project dependencies
    // Convert URI to file path to determine project root
    let current_file_path = uri_to_path(file_uri).context(format!(
        "[find_definition_location] failed to convert uri {} to path",
        &file_uri
    ))?;
    let project_root = find_project_root(&current_file_path).context(format!(
        "[find_definition_location] cannot find the project root. file_uri: {}",
        &file_uri,
    ))?;

    // Look up symbol in the dependency cache
    // The symbol_index maps (project_root, symbol_name) -> Vec<file_locations>
    let symbol_name = usage_node.utf8_text(source.as_bytes()).context(format!(
        "[find_definition_location] cannot get the symbol name for node {:#?}",
        usage_node
    ))?;
    let symbol_key = (project_root.clone(), symbol_name.to_string());
    let symbol_locations = dependency_cache
        .symbol_index
        .get(&symbol_key)
        .context(format!(
            "[find_definition_location] cannot get location for symbol key: {:#?}",
            symbol_key
        ))?;

    // Search through each potential location
    // There might be multiple files containing the same symbol name
    for file_path in symbol_locations.iter() {
        if let Some(external_location) = search_in_external_file_for_location(file_path, usage_node)
        {
            return Ok(external_location);
        }
    }

    Err(anyhow!("[find_definition_location] invalid data"))
}

fn search_local_definitions_for_location(
    tree: &Tree,
    source: &str,
    file_uri: &str,
    usage_node: &Node,
) -> Option<Location> {
    let definition_node = search_local_definitions(tree, source, usage_node)?;

    node_to_lsp_location(&definition_node, file_uri)
}

fn search_local_definitions<'a>(
    tree: &'a Tree,
    source: &str,
    usage_node: &Node<'a>,
) -> Option<Node<'a>> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_type = determine_symbol_type_from_context(tree, usage_node, source).ok()?;

    let query_text = match symbol_type {
        SymbolType::Function => r#"(method_declaration name: (identifier) @name)"#,
        SymbolType::Class => r#"(class_declaration name: (identifier) @name)"#,
        SymbolType::Interface => r#"(interface_declaration name: (identifier) @name)"#,
        SymbolType::Method => r#"(method_declaration name: (identifier) @name)"#,
        SymbolType::Field => {
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#
        }
        SymbolType::Variable => {
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
            "#
        }
        SymbolType::Parameter => r#"(formal_parameter name: (identifier) @name)"#,
        SymbolType::Enum => r#"(enum_declaration name: (identifier) @name)"#,
        _ => return None,
    };

    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut candidates = Vec::new();
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let node = capture.node;
                let node_text = node.utf8_text(source.as_bytes()).unwrap();
                if node_text == symbol_name {
                    candidates.push(node.parent().unwrap());
                };
            }
        });

    if matches!(symbol_type, SymbolType::Variable | SymbolType::Parameter) {
        // The nearest variable declaration is more likely to be correct (but not guaranteed)
        find_closest_declaration(usage_node, &candidates)
    } else {
        candidates.into_iter().next()
    }
}

fn find_closest_declaration<'a>(usage_node: &Node, candidates: &[Node<'a>]) -> Option<Node<'a>> {
    let mut best_candidate = None;
    let mut best_scope_distance = usize::MAX;

    for candidate in candidates {
        if let Some(distance) = calculate_scope_distance(usage_node, candidate) {
            if distance < best_scope_distance {
                best_scope_distance = distance;
                best_candidate = Some(*candidate);
            }
        }
    }

    best_candidate
}

fn calculate_scope_distance(usage_node: &Node, declaration_node: &Node) -> Option<usize> {
    // Check if declaration is in scope of usage
    if !is_in_scope(usage_node, declaration_node) {
        return None;
    }

    // Calculate nesting distance
    let usage_depth = get_nesting_depth(usage_node);
    let decl_depth = get_nesting_depth(declaration_node);

    // Prefer closer scopes (higher depth difference means closer)
    Some(usage_depth.saturating_sub(decl_depth))
}

fn is_in_scope(usage_node: &Node, declaration_node: &Node) -> bool {
    // For formal parameters, check if usage is in the same method
    if let Some(decl_method) = find_containing_method(declaration_node) {
        if let Some(usage_method) = find_containing_method(usage_node) {
            return decl_method.id() == usage_method.id();
        }
    }

    // For local variables, check if declaration comes before usage in same block
    if let Some(decl_block) = find_containing_block(declaration_node) {
        if let Some(usage_block) = find_containing_block(usage_node) {
            if decl_block.id() == usage_block.id() {
                // Check if declaration comes before usage
                return declaration_node.start_position() < usage_node.start_position();
            }
        }
    }

    false
}

fn find_containing_method<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_declaration" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn find_containing_block<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "block" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn get_nesting_depth(node: &Node) -> usize {
    let mut depth = 0;
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "block" | "method_declaration" | "class_declaration"
        ) {
            depth += 1;
        }
        current = parent.parent();
    }
    depth
}

fn search_in_external_file_for_location(
    file_path: &PathBuf,
    usage_node: &Node,
) -> Option<Location> {
    // Step 1: Read the external file
    let content = std::fs::read_to_string(file_path).ok()?;

    // Step 2: Determine language and create parser
    let language = detect_language_from_path(file_path)?;
    let mut parser = create_parser_for_language(language)?;

    // Step 3: Parse the external file
    let tree = parser.parse(&content, None)?;

    // Step 4: Search for the symbol definition in the external tree
    let definition_node = search_local_definitions(&tree, &content, usage_node)?;

    // Step 5: Convert file path to URI
    let file_uri = path_to_file_uri(file_path)?;

    // Step 6: Convert node to Location
    node_to_lsp_location(&definition_node, &file_uri)
}

fn node_to_lsp_location(node: &Node, file_uri: &str) -> Option<Location> {
    let start_pos = node.start_position();
    let end_pos = node.end_position();

    let range = Range {
        start: Position {
            line: start_pos.row as u32,
            character: start_pos.column as u32,
        },
        end: Position {
            line: end_pos.row as u32,
            character: end_pos.column as u32,
        },
    };

    let uri = Url::parse(file_uri).ok()?;
    Some(Location { uri, range })
}
