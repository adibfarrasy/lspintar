use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use log::debug;
use tower_lsp::lsp_types::{Location, Position, Range, Url};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::languages::groovy::symbols::SymbolType;

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

pub fn node_to_lsp_location(node: &Node, file_uri: &str) -> Option<Location> {
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

pub fn get_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration name: (identifier) @name)
            (interface_declaration name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
        "#,
        ),
        SymbolType::Class => Some(r#"(class_declaration name: (identifier) @name)"#),
        SymbolType::Interface => Some(r#"(interface_declaration name: (identifier) @name)"#),
        SymbolType::Enum => Some(r#"(enum_declaration name: (identifier) @name)"#),
        SymbolType::Function | SymbolType::Method => {
            Some(r#"(method_declaration name: (identifier) @name)"#)
        }
        SymbolType::Field => Some(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        ),
        SymbolType::Variable => Some(
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
        "#,
        ),
        SymbolType::Parameter => Some(r#"(formal_parameter name: (identifier) @name)"#),
        _ => None,
    }
}

pub fn find_definition_candidates<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    query_text: &str,
) -> Option<Vec<Node<'a>>> {
    let query = Query::new(&tree.language(), query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            for capture in query_match.captures {
                let node_text = capture.node.utf8_text(source.as_bytes()).unwrap();

                debug!("node_text: {}", node_text);
                if node_text == symbol_name {
                    candidates.push(capture.node.parent().unwrap());
                }
            }
        });

    Some(candidates)
}

pub fn determine_symbol_type_from_context(
    tree: &Tree,
    node: &Node,
    source: &str,
) -> Result<SymbolType> {
    let node_text = node.utf8_text(source.as_bytes())?;

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

        ; Imports
        (import_declaration
          (fully_qualified_name) @import_name) 

        (import_declaration
          (wildcard_import) @import_name) 
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
                        "import_name" => SymbolType::Package,
                        _ => SymbolType::Variable,
                    };

                    result = Ok(symbol);
                    found = true;
                }
            }
        });

    result
}

pub fn search_definition<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    symbol_type: SymbolType,
) -> Option<Node<'a>> {
    let query_text = get_query_for_symbol_type(&symbol_type)?;

    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;

    candidates.into_iter().next()
}
