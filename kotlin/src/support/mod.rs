use lsp_core::{
    language_support::{IdentResult, LanguageSupport, ParameterResult, ParseResult},
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, get_node_at_position, node_contains_position},
};
use std::{cell::RefCell, fs, path::Path, sync::Mutex};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::{
    constants::{KOTLIN_IMPLICIT_IMPORTS, PARSE_TIMEOUT_MICROS},
    support::queries::{
        DECLARES_VARIABLE_QUERY, GET_ANNOTATIONS_QUERY, GET_EXTENDS_QUERY, GET_FIELD_RETURN_QUERY,
        GET_FIELD_SHORT_NAME_QUERY, GET_FUNCTION_RETURN_QUERY, GET_IMPLEMENTS_QUERY,
        GET_IMPORTS_QUERY, GET_KDOC_QUERY, GET_MODIFIERS_QUERY, GET_PACKAGE_NAME_QUERY,
        GET_PARAMETERS_QUERY, GET_SHORT_NAME_QUERY, IDENT_QUERY,
    },
};

mod queries;

pub struct KotlinSupport;

impl KotlinSupport {
    pub fn new() -> Self {
        Self
    }

    fn try_extract_ident_result(
        &self,
        query: &Query,
        match_: &QueryMatch,
        content: &str,
        position: &Position,
        name: &str,
        qual: Option<&str>,
    ) -> Option<IdentResult> {
        let name_idx = query.capture_index_for_name(name);
        let name_cap = match_.captures.iter().find(|c| Some(c.index) == name_idx)?;

        // Check if position is on the name
        if node_contains_position(&name_cap.node, position) {
            let ident = name_cap
                .node
                .utf8_text(content.as_bytes())
                .ok()?
                .to_string();
            let qualifier = if let Some(qual_name) = qual {
                let qual_idx = query.capture_index_for_name(qual_name);
                match_
                    .captures
                    .iter()
                    .find(|c| Some(c.index) == qual_idx)
                    .and_then(|cap| cap.node.utf8_text(content.as_bytes()).ok())
                    .map(|s| {
                        let mut result = s.to_string();
                        loop {
                            let new_result = regex::Regex::new(r"\([^()]*\)")
                                .unwrap()
                                .replace_all(&result, "")
                                .to_string();
                            if new_result == result {
                                break;
                            }
                            result = new_result;
                        }
                        result.replace(".", "#").replace("new ", "")
                    })
            } else {
                None
            };
            return Some((ident, qualifier));
        }

        // Check if position is on the qualifier
        if let Some(qual_name) = qual {
            let qual_idx = query.capture_index_for_name(qual_name);
            if let Some(qual_cap) = match_.captures.iter().find(|c| Some(c.index) == qual_idx) {
                if node_contains_position(&qual_cap.node, position) {
                    if qual_cap.node.kind() == "identifier"
                        || qual_cap.node.kind() == "type_identifier"
                    {
                        let qual_text = qual_cap
                            .node
                            .utf8_text(content.as_bytes())
                            .ok()?
                            .to_string();
                        return Some((qual_text, None));
                    }
                    return self.get_ident_within_node(&qual_cap.node, content, position);
                }
            }
        }

        None
    }

    fn get_ident_within_node(
        &self,
        node: &Node,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        self.find_ident_at_position_impl(*node, content, position)
    }

    fn find_ident_at_position_impl(
        &self,
        root: Node,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let mut cursor = QueryCursor::new();
        let mut result = None;

        cursor
            .matches(&IDENT_QUERY, root, content.as_bytes())
            .for_each(|m| {
                if result.is_some() {
                    return;
                }

                vec![
                    ("trivial_case", None),
                    ("nav_name", Some("nav_qualifier")),
                    ("arg_name", None),
                    ("var_decl", None),
                    ("this_method_name", Some("this_qualifier")),
                    ("constructor_type", None),
                    ("type_arg", None),
                    ("cast_type", None),
                    ("class_name", None),
                    ("interface_name", None),
                    ("function_name", None),
                    ("property_name", None),
                    ("super_interfaces", None),
                    ("superclass", None),
                    ("return_name", None),
                    ("annotation", None),
                ]
                .into_iter()
                .for_each(|(name, qual)| {
                    if let Some(r) = self.try_extract_ident_result(
                        &IDENT_QUERY,
                        &m,
                        content,
                        position,
                        name,
                        qual,
                    ) {
                        result = Some(r);
                        return;
                    }
                });

                if let Some(r) = self.try_extract_import_ident(&IDENT_QUERY, &m, content, position)
                {
                    result = Some(r);
                    return;
                }
            });

        result
    }

    fn try_extract_import_ident(
        &self,
        query: &Query,
        match_: &QueryMatch,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let full_import_idx = query.capture_index_for_name("full_import")?;
        let full_import_cap = match_
            .captures
            .iter()
            .find(|c| c.index == full_import_idx)?;

        let full_import_node = full_import_cap.node;

        if !node_contains_position(&full_import_node, position) {
            return None;
        }

        let full_text = full_import_node.utf8_text(content.as_bytes()).ok()?;

        let parts: Vec<&str> = full_text.split('.').collect();

        if parts.is_empty() {
            return None;
        }

        let name = parts.last()?.to_string();

        let qualifier = if parts.len() > 1 {
            Some(parts[..parts.len() - 1].join("."))
        } else {
            None
        };

        Some((name, qualifier))
    }

    fn find_in_current_scope(
        &self,
        scope_node: Node,
        content: &str,
        var_name: &str,
        reference_byte: usize,
    ) -> Option<(Option<String>, Position)> {
        let mut cursor = scope_node.walk();
        if !cursor.goto_first_child() {
            return None;
        }
        loop {
            let child = cursor.node();
            // Only process nodes that come before the reference
            if child.start_byte() >= reference_byte {
                break;
            }
            match child.kind() {
                "property_declaration" => {
                    let mut var_cursor = child.walk();
                    for var_child in child.children(&mut var_cursor) {
                        if var_child.kind() == "variable_declaration" {
                            if self.declares_variable(var_child, content, var_name) {
                                // Try to get explicit type
                                let mut type_cursor = var_child.walk();
                                for type_child in var_child.children(&mut type_cursor) {
                                    if type_child.kind() == "user_type"
                                        || type_child.kind() == "nullable_type"
                                    {
                                        let type_name = type_child
                                            .utf8_text(content.as_bytes())
                                            .ok()
                                            .map(|s| s.to_string());

                                        let (_, var_position) = ts_helper::get_one_with_position(
                                            &child,
                                            content,
                                            &DECLARES_VARIABLE_QUERY,
                                        )?;

                                        return Some((type_name, var_position));
                                    }
                                }

                                // Infer from value if no explicit type
                                let mut value_cursor = child.walk();
                                for value_child in child.children(&mut value_cursor) {
                                    if value_child.kind() == "call_expression" {
                                        let type_name =
                                            self.infer_type_from_value(value_child, content);
                                        let identifier = var_child.child_by_field_name("name")?;
                                        let var_position = Position {
                                            line: identifier.start_position().row as u32,
                                            character: identifier.start_position().column as u32,
                                        };
                                        return Some((type_name, var_position));
                                    }
                                }
                            }
                        }
                    }
                }
                "class_parameter" | "parameter" => {
                    if self.declares_variable(child, content, var_name) {
                        let type_node = child.child_by_field_name("type")?;
                        let type_name = type_node
                            .utf8_text(content.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                        let identifier = child.child_by_field_name("name")?;
                        let var_position = Position {
                            line: identifier.start_position().row as u32,
                            character: identifier.start_position().column as u32,
                        };
                        return Some((type_name, var_position));
                    }
                }
                "variable_declaration" => {
                    if self.declares_variable(child, content, var_name) {
                        if let Some(type_node) = child.child_by_field_name("type") {
                            let type_name = type_node
                                .utf8_text(content.as_bytes())
                                .ok()
                                .map(|s| s.to_string());
                            let identifier = child.child_by_field_name("name")?;
                            let var_position = Position {
                                line: identifier.start_position().row as u32,
                                character: identifier.start_position().column as u32,
                            };
                            return Some((type_name, var_position));
                        }
                    }
                }
                "statements" | "function_body" | "lambda_literal" | "parameters"
                | "lambda_parameters" => {
                    if let Some(result) =
                        self.find_in_current_scope(child, content, var_name, reference_byte)
                    {
                        return Some(result);
                    }
                }
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        None
    }

    fn infer_type_from_value(&self, value_node: Node, content: &str) -> Option<String> {
        match value_node.kind() {
            "call_expression" => {
                if let Some(ident_node) = value_node.child(0) {
                    if ident_node.kind() == "identifier" {
                        return ident_node
                            .utf8_text(content.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn declares_variable(&self, node: Node, content: &str, var_name: &str) -> bool {
        let names = ts_helper::get_many(&node, content, &DECLARES_VARIABLE_QUERY, Some(1));
        names.iter().any(|name| name == var_name)
    }

    fn parse_argument_list(&self, arg_list_node: &Node, content: &str) -> Vec<(String, Position)> {
        let mut args = Vec::new();
        let mut cursor = arg_list_node.walk();

        for child in arg_list_node.children(&mut cursor) {
            if child.is_named() {
                if let Ok(arg_text) = child.utf8_text(content.as_bytes()) {
                    let arg_name = arg_text.to_string();
                    let position = Position {
                        line: child.start_position().row as u32,
                        character: child.start_position().column as u32,
                    };
                    args.push((arg_name, position));
                }
            }
        }

        args
    }

    fn extract_method_parameters(&self, method_node: &Node, content: &str) -> Vec<String> {
        let mut cursor = method_node.walk();
        for child in method_node.children(&mut cursor) {
            if child.kind() == "parameters" {
                return self.parse_parameter_list(&child, content);
            }
        }
        vec![]
    }

    fn parse_parameter_list(&self, params_node: &Node, content: &str) -> Vec<String> {
        let mut params = Vec::new();
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter" {
                if let Some(type_node) = child.child_by_field_name("type") {
                    if let Ok(type_name) = type_node.utf8_text(content.as_bytes()) {
                        params.push(type_name.to_string());
                    }
                }
            }
        }
        params
    }

    fn parse_parameter(&self, param: &str) -> ParameterResult {
        let param = param.trim();
        param
            .split_once(':')
            .map(|(name, rest)| {
                if let Some((arg, default)) = rest.split_once('=') {
                    (
                        name.trim().to_string(),
                        Some(arg.trim().to_string()),
                        Some(default.trim().to_string()),
                    )
                } else {
                    (name.trim().to_string(), Some(rest.trim().to_string()), None)
                }
            })
            .unwrap_or((param.to_string(), None, None))
    }
}

impl LanguageSupport for KotlinSupport {
    fn get_language(&self) -> Language {
        Language::Kotlin
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_kotlin::language()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        thread_local! {
            static PARSER: RefCell<Parser> = RefCell::new({
                let mut p = Parser::new();
                p.set_language(&tree_sitter_kotlin::language()).unwrap();
                p.set_timeout_micros(PARSE_TIMEOUT_MICROS);
                p
            });
        }
        PARSER.with(|p| {
            p.borrow_mut()
                .parse(content, None)
                .map(|tree| (tree, content.to_string()))
        })
    }

    fn get_range(&self, node: &Node) -> Option<Range> {
        let range = node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_ident_range(&self, node: &Node) -> Option<Range> {
        let ident_node = match node.kind() {
            "class_declaration" | "function_declaration" => node.child_by_field_name("name")?,
            "field_declaration" | "constant_declaration" => {
                let declarator = node
                    .children(&mut node.walk())
                    .find(|n| n.kind() == "variable_declarator")?;
                declarator.child_by_field_name("name")?
            }
            _ => node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier")?,
        };

        let range = ident_node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_package_name(&self, tree: &Tree, content: &str) -> Option<String> {
        ts_helper::get_one(&tree.root_node(), content, &GET_PACKAGE_NAME_QUERY)
    }

    fn get_kind(&self, node: &Node) -> Option<NodeKind> {
        match node.kind() {
            "class_declaration" => {
                if let Some(body) = node.child_by_field_name("body") {
                    match body.kind() {
                        "enum_class_body" => Some(NodeKind::Enum),
                        _ => Some(NodeKind::Class),
                    }
                } else {
                    Some(NodeKind::Class)
                }
            }
            "interface_declaration" => Some(NodeKind::Interface),
            "function_declaration" => Some(NodeKind::Function),
            "property_declaration" => Some(NodeKind::Field),
            "class_parameter" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "binding_pattern_kind" {
                        return Some(NodeKind::Field);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        let node_kind = self.get_kind(node);

        match node_kind {
            Some(NodeKind::Field) => ts_helper::get_one(node, source, &GET_FIELD_SHORT_NAME_QUERY),
            Some(_) => ts_helper::get_one(node, source, &GET_SHORT_NAME_QUERY),
            None => None,
        }
    }

    fn get_extends(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_EXTENDS_QUERY)
    }

    fn get_implements(&self, node: &Node, source: &str) -> Vec<String> {
        ts_helper::get_many(node, source, &GET_IMPLEMENTS_QUERY, Some(1))
    }

    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String> {
        match self.get_kind(node) {
            Some(_) => ts_helper::get_one(node, source, &GET_MODIFIERS_QUERY)
                .map(|line| {
                    line.split_whitespace()
                        .into_iter()
                        .map(|m| m.to_string())
                        .collect()
                })
                .unwrap_or(vec![]),
            None => Vec::new(),
        }
    }

    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String> {
        let node_kind = self.get_kind(node);

        match node_kind {
            Some(_) => ts_helper::get_many(node, source, &GET_ANNOTATIONS_QUERY, Some(1))
                .into_iter()
                .collect(),
            None => Vec::new(),
        }
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_KDOC_QUERY)
    }

    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>> {
        if let Some(NodeKind::Function) = self.get_kind(node) {
            let params = ts_helper::get_many(node, source, &GET_PARAMETERS_QUERY, Some(1))
                .into_iter()
                .map(|p| self.parse_parameter(&p))
                .collect();
            Some(params)
        } else {
            None
        }
    }

    fn get_return(&self, node: &Node, source: &str) -> Option<String> {
        let node_kind = self.get_kind(node);

        match node_kind {
            Some(NodeKind::Field) => ts_helper::get_one(node, source, &GET_FIELD_RETURN_QUERY),
            Some(NodeKind::Function) => {
                ts_helper::get_one(node, source, &GET_FUNCTION_RETURN_QUERY)
            }
            _ => None,
        }
    }

    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String> {
        let explicit_imports =
            ts_helper::get_many(&tree.root_node(), source, &GET_IMPORTS_QUERY, None)
                .into_iter()
                .map(|i| {
                    i.strip_prefix("import ")
                        .unwrap_or_default()
                        .trim_end_matches(';')
                        .trim()
                        .to_string()
                })
                .collect::<Vec<String>>();

        KOTLIN_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(explicit_imports)
            .collect()
    }

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        let query_text = r#"
        [
          (property_declaration (variable_declaration type: (user_type) @identifier))
          (property_declaration (variable_declaration type: (nullable_type) @identifier))
          (variable_declaration type: (user_type) @identifier)
          (variable_declaration type: (nullable_type) @identifier)
          (parameter type: (user_type) @identifier)
          (parameter type: (nullable_type) @identifier)
          (class_parameter type: (user_type) @identifier)
          (class_parameter type: (nullable_type) @identifier)
          (interface_declaration name: (type_identifier) @identifier)
          (class_declaration name: (type_identifier) @identifier)
        ]
        "#;
        let query = Query::new(&self.get_ts_language(), query_text).ok()?;
        let mut result = None;
        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, node, content.as_bytes())
            .find(|match_| {
                for capture in match_.captures.iter() {
                    let node = capture.node;
                    if node_contains_position(&node, position) {
                        let ident_name = node
                            .utf8_text(content.as_bytes())
                            .unwrap_or_default()
                            .to_string();
                        result = Some(ident_name);
                    }
                }
                result.is_some()
            });
        result
    }

    fn find_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        self.find_ident_at_position_impl(tree.root_node(), content, position)
    }

    fn find_variable_type(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<String> {
        self.find_variable_declaration(tree, content, var_name, position)
            .map(|(type_name, _)| type_name)?
    }

    fn extract_call_arguments(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<Vec<(String, Position)>> {
        let point = Point::new(position.line as usize, position.character as usize);
        let node = tree.root_node().descendant_for_point_range(point, point)?;

        let mut current = node;
        loop {
            if current.kind() == "call_expression" {
                let query_str = r#"
                    (call_expression
                      (call_suffix
                        (value_arguments) @args))
                "#;

                let query =
                    tree_sitter::Query::new(&tree_sitter_kotlin::language(), query_str).ok()?;
                let mut cursor = QueryCursor::new();
                let mut result = None;

                cursor
                    .matches(&query, current, content.as_bytes())
                    .find(|match_| {
                        for capture in match_.captures.iter() {
                            let args_node = capture.node;
                            result = Some(self.parse_argument_list(&args_node, content));
                        }
                        result.is_some()
                    });

                return result.or(Some(vec![]));
            }
            current = match current.parent() {
                Some(p) => p,
                None => return None,
            };
        }
    }

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String> {
        let point = Point::new(position.line as usize, position.character as usize);
        let mut node = tree.root_node().descendant_for_point_range(point, point)?;

        loop {
            match node.kind() {
                "collection_literal" => return Some("List".to_string()),

                "decimal_integer_literal" | "hex_literal" | "bin_literal" => {
                    if let Ok(_) = node.utf8_text(content.as_bytes()) {
                        if let Some(parent) = node.parent() {
                            match parent.kind() {
                                "long_literal" => {
                                    return Some("Long".to_string());
                                }
                                "unsigned_literal" => {
                                    if let Ok(parent_text) = parent.utf8_text(content.as_bytes()) {
                                        let parent_lower = parent_text.trim().to_lowercase();
                                        if parent_lower.ends_with("ul") {
                                            return Some("ULong".to_string());
                                        }
                                        return Some("UInt".to_string());
                                    }
                                    return Some("UInt".to_string());
                                }
                                _ => {
                                    return Some("Int".to_string());
                                }
                            }
                        }

                        return Some("Int".to_string());
                    }
                    return Some("Int".to_string());
                }

                "long_literal" => {
                    return Some("Long".to_string());
                }

                "unsigned_literal" => {
                    if let Ok(text) = node.utf8_text(content.as_bytes()) {
                        let lower = text.to_lowercase();
                        if lower.ends_with("ul") {
                            return Some("ULong".to_string());
                        }
                        return Some("UInt".to_string());
                    }
                    return Some("UInt".to_string());
                }

                "real_literal" => {
                    if let Ok(text) = node.utf8_text(content.as_bytes()) {
                        let lower = text.to_lowercase();
                        if lower.ends_with('f') || lower.ends_with('F') {
                            return Some("Float".to_string());
                        }
                        // In Kotlin, real literals without suffix are Double
                        return Some("Double".to_string());
                    }
                    return Some("Double".to_string());
                }

                "boolean_literal" => return Some("Boolean".to_string()),

                "character_literal" => return Some("Char".to_string()),

                "null_literal" => return None,

                "string_literal" => return Some("String".to_string()),

                _ => {}
            }

            node = match node.parent() {
                Some(p) => p,
                None => return None,
            };
        }
    }

    fn get_method_receiver_and_params(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<(String, Vec<String>)> {
        let query_text = r#"
        [
            (class_declaration 
            name: (type_identifier) @receiver
            body: (class_body (function_declaration) @method))
            (interface_declaration 
            name: (type_identifier) @receiver
            body: (interface_body (function_declaration) @method))
            (class_declaration 
            name: (type_identifier) @receiver
            body: (enum_class_body (function_declaration) @method))
            (object_declaration 
            name: (type_identifier) @receiver
            body: (class_body (function_declaration) @method))
        ]
        "#;
        let query = Query::new(&self.get_ts_language(), query_text).ok()?;

        let method_idx = query.capture_index_for_name("method");
        let receiver_idx = query.capture_index_for_name("receiver");

        if method_idx.is_none() || receiver_idx.is_none() {
            return None;
        }

        let method_idx = method_idx.unwrap();
        let receiver_idx = receiver_idx.unwrap();
        let mut result = None;
        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, node, content.as_bytes())
            .find(|match_| {
                let Some(method_capture) = match_.captures.iter().find(|c| c.index == method_idx)
                else {
                    return false;
                };

                if node_contains_position(&method_capture.node, position) {
                    let Some(receiver_capture) =
                        match_.captures.iter().find(|c| c.index == receiver_idx)
                    else {
                        return false;
                    };
                    if let Ok(text) = receiver_capture.node.utf8_text(content.as_bytes()) {
                        let params = self.extract_method_parameters(&method_capture.node, content);
                        result = Some((text.to_string(), params));
                        return true;
                    }
                }
                false
            });

        result
    }

    fn find_variable_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(Option<String>, Position)> {
        let mut current_node = get_node_at_position(tree, content, position)?;
        if var_name == "this" {
            let mut node = current_node;
            while let Some(parent) = node.parent() {
                if parent.kind() == "class_declaration" {
                    let type_node = parent.child_by_field_name("name")?;
                    let pos = Position {
                        line: type_node.start_position().row as u32,
                        character: type_node.start_position().column as u32,
                    };
                    let name = self
                        .get_ident_within_node(&parent, content, &pos)
                        .map(|(name, _)| name)?;
                    return Some((Some(name), pos));
                }
                node = parent;
            }
            return None;
        }

        let reference_byte = current_node.start_byte();
        loop {
            if let Some(result) =
                self.find_in_current_scope(current_node, content, var_name, reference_byte)
            {
                return Some(result);
            }
            if let Some(parent) = current_node.parent() {
                current_node = parent;
            } else {
                break;
            }
        }
        None
    }
}

#[allow(dead_code)]
mod tests {
    use tower_lsp::lsp_types::Position;
    use tree_sitter::Node;

    mod extract_call_arguments;
    mod find_ident_at_position;
    mod find_variable_type;
    mod get_imports;
    mod get_indexer_data;
    mod get_literal_type;
    mod get_method_receiver_and_params;

    fn find_position(content: &str, marker: &str) -> Position {
        content
            .lines()
            .enumerate()
            .find_map(|(line_num, line)| {
                line.find(marker)
                    .map(|col| Position::new(line_num as u32, col as u32))
            })
            .expect(&format!("Marker '{}' not found", marker))
    }

    fn find_node_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_node_by_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }
}
