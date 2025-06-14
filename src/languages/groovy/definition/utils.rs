use std::{
    fs::{self, read_to_string},
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use log::debug;
use tower_lsp::lsp_types::Location;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::{
    symbols::SymbolType,
    utils::{find_project_root, node_to_lsp_location, uri_to_path, uri_to_tree},
};

pub fn get_declaration_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration name: (identifier) @name)
            (interface_declaration name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
        "#,
        ),
        SymbolType::SuperClass => Some(r#"(class_declaration name: (identifier) @name)"#),
        SymbolType::SuperInterface => Some(r#"(interface_declaration name: (identifier) @name)"#),
        SymbolType::MethodCall => Some(r#"(method_declaration name: (identifier) @name)"#),
        SymbolType::FieldUsage => Some(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        ),
        SymbolType::VariableUsage => Some(
            r#"
            (variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
        "#,
        ),
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
            name: (identifier) @var_decl))

        ; Field declarations  
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_decl))

        ; Class declarations
        (class_declaration
          name: (identifier) @class_decl)

        ; Interface declarations
        (interface_declaration
          name: (identifier) @interface_decl)

        ; Method declarations
        (method_declaration
          name: (identifier) @method_decl)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_decl)

        ; Parameters
        (formal_parameter
          name: (identifier) @param_decl)

        ; USAGES
        (field_access field: (identifier) @field_usage)

        (method_invocation name: (identifier) @method_usage)

        (argument_list (identifier) @var_usage)

        (assignment_expression left: (identifier) @var_usage)

        (assignment_expression right: (identifier) @var_usage)

        ; Interface
        (class_declaration
          interfaces: (super_interfaces
            (type_list (type_identifier) @super_interface)))
        (interface_declaration
          (extends_interfaces
            (type_list (type_identifier) @super_interface)))

        ; Superclass
        (class_declaration
          superclass: (superclass
            (type_identifier) @super_class))

        ; Type identifiers
        (type_identifier) @type_name

        ; Imports
        (import_declaration
          (scoped_identifier) @import_name) 

        ; Method usage
        (scoped_identifier) @method_usage
    "#;

    let query = Query::new(&tree_sitter_groovy::language(), query_text)
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

                let capture_range = capture.node.range();
                let node_range = node.range();

                if capture_text == node_text && capture_range == node_range {
                    let capture_name = query.capture_names()[capture.index as usize];
                    let symbol = match capture_name {
                        "import_name" => SymbolType::PackageDeclaration,
                        "var_decl" => SymbolType::VariableDeclaration,
                        "field_decl" => SymbolType::FieldDeclaration,
                        "class_decl" => SymbolType::ClassDeclaration,
                        "interface_decl" => SymbolType::InterfaceDeclaration,
                        "method_decl" => SymbolType::MethodDeclaration,
                        "enum_decl" => SymbolType::EnumDeclaration,
                        "param_decl" => SymbolType::ParameterDeclaration,

                        "method_usage" => SymbolType::MethodCall,
                        "type_name" => SymbolType::Type,
                        "super_interface" => SymbolType::SuperInterface,
                        "super_class" => SymbolType::SuperClass,
                        "field_usage" => SymbolType::FieldUsage,
                        "var_usage" => SymbolType::VariableUsage,

                        _ => SymbolType::VariableUsage,
                    };

                    result = Ok(symbol);
                    found = true;
                }
            }
        });

    debug!("node_text: {node_text}, result: {:#?}", result);
    result
}

pub fn search_definition<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    symbol_type: SymbolType,
) -> Option<Node<'a>> {
    let query_text = get_declaration_query_for_symbol_type(&symbol_type)?;

    let candidates = find_definition_candidates(tree, source, symbol_name, query_text)?;

    candidates.into_iter().next()
}

pub fn search_definition_in_project(
    current_file_uri: &str,
    current_source: &str,
    usage_node: &Node,
    other_file_uri: &str,
) -> Option<Location> {
    let current_tree = uri_to_tree(current_file_uri)?;
    let symbol_name = usage_node.utf8_text(current_source.as_bytes()).ok()?;
    let symbol_type =
        determine_symbol_type_from_context(&current_tree, usage_node, current_source).ok()?;

    debug!("other_file_uri: {other_file_uri}");
    let other_tree = uri_to_tree(other_file_uri)?;
    let other_path = uri_to_path(other_file_uri)?;
    let other_source = read_to_string(other_path).ok()?;

    let definition_node = search_definition(&other_tree, &other_source, symbol_name, symbol_type)?;

    return node_to_lsp_location(&definition_node, &other_file_uri);
}

pub fn prepare_symbol_lookup_key(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
) -> Option<(PathBuf, String)> {
    let symbol_bytes = usage_node.utf8_text(source.as_bytes()).ok()?;
    let symbol_name = symbol_bytes.to_string();

    let current_file_path = uri_to_path(file_uri)?;

    let project_root = project_root.or_else(|| find_project_root(&current_file_path))?;

    resolve_through_imports(&symbol_name, source, &project_root)
}

pub fn resolve_through_imports(
    symbol_name: &str,
    source: &str,
    project_root: &PathBuf,
) -> Option<(PathBuf, String)> {
    let query_text = r#"
        (import_declaration
          (scoped_identifier) @import_name) 
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                return; // Already found a match
            }

            for capture in query_match.captures {
                if let Ok(import_text) = capture.node.utf8_text(source.as_bytes()) {
                    debug!("import_text: {import_text}, symbol_name: {symbol_name}");

                    if import_text.ends_with(&format!(".{}", symbol_name)) {
                        result = Some((project_root.clone(), import_text.to_string()));
                        return;
                    }

                    if import_text.ends_with("*") {
                        // TODO: handle wildcard import
                    }
                };
            }
        });

    result
}

pub fn set_start_position(source: &str, usage_node: &Node, file_uri: &str) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;

    let other_source = fs::read_to_string(uri_to_path(file_uri)?).ok()?;

    let query_text = r#"
      (identifier) @name 
    "#;

    let language = tree_sitter_groovy::language();
    let query = Query::new(&language, query_text).ok()?;

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&other_source, None)?;

    let mut cursor = QueryCursor::new();
    let mut result = None;

    cursor
        .matches(&query, tree.root_node(), other_source.as_bytes())
        .for_each(|query_match| {
            if result.is_some() {
                return; // Already found a match
            }

            for capture in query_match.captures {
                if let Ok(name) = capture.node.utf8_text(other_source.as_bytes()) {
                    if name == symbol_name {
                        result = node_to_lsp_location(&capture.node, file_uri)
                    }
                };
            }
        });

    result
}
