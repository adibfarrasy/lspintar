use lsp_core::{
    language_support::{IdentResult, LanguageSupport, ParameterResult, ParseResult},
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, collect_syntax_errors, get_node_at_position, node_contains_position},
};
use std::{cell::RefCell, fs, path::Path};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::{
    constants::KOTLIN_IMPLICIT_IMPORTS,
    support::queries::{
        DECLARES_VARIABLE_QUERY, GET_ANNOTATIONS_QUERY, GET_EXTENDS_QUERY, GET_FIELD_RETURN_QUERY,
        GET_FIELD_SHORT_NAME_QUERY, GET_FUNCTION_RETURN_QUERY, GET_IMPLEMENTS_QUERY,
        GET_IMPORTS_QUERY, GET_KDOC_QUERY, GET_MODIFIERS_QUERY, GET_PACKAGE_NAME_QUERY,
        GET_PARAMETERS_QUERY, GET_SHORT_NAME_QUERY, GET_TYPE_QUERY, IDENT_QUERY,
    },
};

mod queries;

pub struct KotlinSupport;

impl Default for KotlinSupport {
    fn default() -> Self {
        Self::new()
    }
}

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
                        let regex = regex::Regex::new(r"\([^()]*\)").unwrap();
                        loop {
                            let new_result = regex.replace_all(&result, "").to_string();
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
            if let Some(qual_cap) = match_.captures.iter().find(|c| Some(c.index) == qual_idx)
                && node_contains_position(&qual_cap.node, position)
            {
                if qual_cap.node.kind() == "identifier" || qual_cap.node.kind() == "type_identifier"
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
                        m,
                        content,
                        position,
                        name,
                        qual,
                    ) {
                        result = Some(r);
                    }
                });

                if let Some(r) = self.try_extract_import_ident(&IDENT_QUERY, m, content, position) {
                    result = Some(r);
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

    fn traverse_scope_nodes<F>(
        &self,
        scope_node: Node,
        content: &str,
        reference_byte: usize,
        process_node: &mut F,
    ) where
        F: FnMut(Node, &str) -> bool,
    {
        let mut stack = Vec::new();
        stack.push(scope_node);

        while let Some(current_node) = stack.pop() {
            let mut cursor = current_node.walk();
            if !cursor.goto_first_child() {
                continue;
            }

            loop {
                let child = cursor.node();
                if child.start_byte() >= reference_byte {
                    break;
                }

                match child.kind() {
                    "property_declaration"
                    | "class_parameter"
                    | "parameter"
                    | "variable_declaration" => {
                        if !process_node(child, content) {
                            return;
                        }
                    }
                    "statements" | "function_body" | "lambda_literal" | "parameters"
                    | "lambda_parameters" => {
                        stack.push(child);
                    }
                    _ => {}
                }

                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    // Helper function to extract variable info from a node
    fn extract_variable_info(
        &self,
        node: Node,
        content: &str,
        var_name: &str,
    ) -> Option<(Option<String>, Position)> {
        match node.kind() {
            "property_declaration" => {
                let mut var_cursor = node.walk();
                for var_child in node.children(&mut var_cursor) {
                    if var_child.kind() == "variable_declaration"
                        && self.declares_variable(var_child, content, var_name)
                    {
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
                                    &node,
                                    content,
                                    &DECLARES_VARIABLE_QUERY,
                                )?;

                                return Some((type_name, var_position));
                            }
                        }

                        // Infer from value if no explicit type
                        if let Some(value_child) = node.child_by_field_name("value") {
                            let type_name = self.infer_type_from_value(value_child, content);
                            if type_name.is_some() {
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
                None
            }
            "class_parameter" | "parameter" => {
                if self.declares_variable(node, content, var_name) {
                    let type_node = node.child_by_field_name("type")?;
                    let type_name = type_node
                        .utf8_text(content.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                    let identifier = node.child_by_field_name("name")?;
                    let var_position = Position {
                        line: identifier.start_position().row as u32,
                        character: identifier.start_position().column as u32,
                    };
                    Some((type_name, var_position))
                } else {
                    None
                }
            }
            "variable_declaration" => {
                if self.declares_variable(node, content, var_name)
                    && let Some(type_node) = node.child_by_field_name("type")
                {
                    let type_name = type_node
                        .utf8_text(content.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                    let identifier = node.child_by_field_name("name")?;
                    let var_position = Position {
                        line: identifier.start_position().row as u32,
                        character: identifier.start_position().column as u32,
                    };
                    Some((type_name, var_position))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    // Helper function to collect variable names from a node
    fn collect_variable_names(&self, node: Node, content: &str) -> Vec<(String, Option<String>)> {
        let mut results = Vec::new();

        match node.kind() {
            "property_declaration" => {
                let mut var_cursor = node.walk();
                for var_child in node.children(&mut var_cursor) {
                    if var_child.kind() == "variable_declaration" {
                        let names = ts_helper::get_many(
                            &var_child,
                            content,
                            &DECLARES_VARIABLE_QUERY,
                            Some(1),
                        );
                        let mut type_cursor = var_child.walk();
                        let var_type = var_child
                            .children(&mut type_cursor)
                            .find(|c| c.kind() == "user_type" || c.kind() == "nullable_type")
                            .and_then(|t| {
                                t.utf8_text(content.as_bytes()).ok().map(|s| s.to_string())
                            })
                            .or_else(|| {
                                node.child_by_field_name("value")
                                    .and_then(|v| self.infer_type_from_value(v, content))
                            });
                        for name in names {
                            results.push((name, var_type.clone()));
                        }
                    }
                }
            }
            "class_parameter" | "parameter" | "variable_declaration" => {
                let names = ts_helper::get_many(&node, content, &DECLARES_VARIABLE_QUERY, Some(1));
                let var_type = node
                    .child_by_field_name("type")
                    .and_then(|t| t.utf8_text(content.as_bytes()).ok().map(|s| s.to_string()));
                for name in names {
                    results.push((name, var_type.clone()));
                }
            }
            _ => {}
        }

        results
    }

    fn find_in_current_scope(
        &self,
        scope_node: Node,
        content: &str,
        var_name: &str,
        reference_byte: usize,
    ) -> Option<(Option<String>, Position)> {
        let mut result = None;
        let mut process_node = |child: Node, content: &str| -> bool {
            if let Some(info) = self.extract_variable_info(child, content, var_name) {
                result = Some(info);
                false
            } else {
                true
            }
        };

        self.traverse_scope_nodes(scope_node, content, reference_byte, &mut process_node);
        result
    }

    fn collect_declarations_in_scope(
        &self,
        scope_node: Node,
        content: &str,
        reference_byte: usize,
        results: &mut Vec<(String, Option<String>)>,
    ) {
        let mut process_node = |child: Node, content: &str| -> bool {
            let names = self.collect_variable_names(child, content);
            results.extend(names);
            true
        };

        self.traverse_scope_nodes(scope_node, content, reference_byte, &mut process_node);
    }

    /// Extracts a `#`-separated chain qualifier from a Kotlin call expression.
    /// Returns `None` if the expression is not a supported chain pattern.
    /// Examples:
    ///   `Bar.create()`     → `Some("Bar#create")`
    ///   `foo.bar().baz()`  → `Some("foo#bar#baz")`
    fn extract_invocation_chain(node: &Node, content: &str) -> Option<String> {
        match node.kind() {
            "identifier" => node
                .utf8_text(content.as_bytes())
                .ok()
                .map(|s| s.to_string()),
            "navigation_expression" => {
                // Standalone property access: it.name
                // navigation_expression = _expression + navigation_suffix
                let receiver = node.child(0)?;
                let nav_suffix = node.child(1)?;
                let field_name_node = nav_suffix.named_child(0)?;
                let field_name = field_name_node.utf8_text(content.as_bytes()).ok()?;
                let receiver_chain = Self::extract_invocation_chain(&receiver, content)?;
                Some(format!("{}#{}", receiver_chain, field_name))
            }
            "call_expression" => {
                // call_expression = _expression + call_suffix
                // If child(0) is a navigation_expression, it's a qualified call: receiver.method()
                // Otherwise it might be a simple call: foo()
                let first = node.child(0)?;
                let chain = match first.kind() {
                    "navigation_expression" => {
                        // navigation_expression = _expression + navigation_suffix
                        let receiver = first.child(0)?;
                        let nav_suffix = first.child(1)?;
                        // navigation_suffix = _member_access_operator + identifier (or similar)
                        // The operator (. or ?.) is anonymous; named_child(0) is the identifier
                        let method_name_node = nav_suffix.named_child(0)?;
                        let method_name = method_name_node.utf8_text(content.as_bytes()).ok()?;
                        let receiver_chain_raw =
                            Self::extract_invocation_chain(&receiver, content)?;
                        // Strip lambda body info from the receiver chain to avoid
                        // propagating it into outer chains where it does not apply.
                        let receiver_chain =
                            if let Some(idx) = receiver_chain_raw.find("__lb__") {
                                receiver_chain_raw[..idx].to_string()
                            } else {
                                receiver_chain_raw
                            };
                        format!("{}#{}", receiver_chain, method_name)
                    }
                    "identifier" => {
                        // simple call: foo()
                        first.utf8_text(content.as_bytes()).ok()?.to_string()
                    }
                    _ => return None,
                };
                // If the call has a trailing lambda, encode the body chain.
                if let Some(body_info) = Self::extract_lambda_body_chain(node, content) {
                    Some(format!("{}__lb__{}", chain, body_info))
                } else {
                    Some(chain)
                }
            }
            _ => None,
        }
    }

    /// If `call_expr` has a trailing lambda argument, returns `"param|body_chain"`.
    /// Returns `None` when no lambda is present or the body is too complex to encode.
    fn extract_lambda_body_chain(call_expr: &Node, content: &str) -> Option<String> {
        // call_suffix → annotated_lambda → lambda_literal
        let mut cur = call_expr.walk();
        let call_suffix = call_expr
            .children(&mut cur)
            .find(|n| n.kind() == "call_suffix")?;

        let mut sc = call_suffix.walk();
        let annotated_lambda = call_suffix
            .children(&mut sc)
            .find(|n| n.kind() == "annotated_lambda")?;

        let mut ac = annotated_lambda.walk();
        let lambda_literal = annotated_lambda
            .children(&mut ac)
            .find(|n| n.kind() == "lambda_literal")?;

        let mut lc = lambda_literal.walk();
        let lambda_children: Vec<_> = lambda_literal.children(&mut lc).collect();

        // Lambda param name: from lambda_parameters if present, otherwise "it" (implicit).
        let param_name = lambda_children
            .iter()
            .find(|n| n.kind() == "lambda_parameters")
            .and_then(|lp| {
                let mut pc = lp.walk();
                lp.named_children(&mut pc)
                    .find(|n| n.kind() == "variable_declaration")
            })
            .and_then(|vd| vd.child_by_field_name("name"))
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "it".to_string());

        // Lambda body: the statements node, last named child.
        let statements = lambda_children
            .iter()
            .find(|n| n.kind() == "statements")?;
        let mut stc = statements.walk();
        let last_stmt = statements.named_children(&mut stc).last()?;

        let body_chain = Self::extract_invocation_chain(&last_stmt, content)?;
        Some(format!("{}|{}", param_name, body_chain))
    }

    fn infer_type_from_value(&self, value_node: Node, content: &str) -> Option<String> {
        match value_node.kind() {
            "call_expression" => Self::extract_invocation_chain(&value_node, content),
            "string_literal" => Some("String".to_string()),
            "decimal_integer_literal" => Some("Int".to_string()),
            "long_literal" => Some("Long".to_string()),
            "real_literal" => {
                if let Ok(text) = value_node.utf8_text(content.as_bytes()) {
                    if text.to_lowercase().ends_with('f') {
                        return Some("Float".to_string());
                    }
                }
                Some("Double".to_string())
            }
            "boolean_literal" => Some("Boolean".to_string()),
            "character_literal" => Some("Char".to_string()),
            _ => None,
        }
    }

    fn declares_variable(&self, node: Node, content: &str, var_name: &str) -> bool {
        let names = ts_helper::get_many(&node, content, &DECLARES_VARIABLE_QUERY, Some(1));
        names.iter().any(|name| name == var_name)
    }

    fn parse_argument_list(&self, arg_list_node: &Node, content: &str) -> Vec<(String, Position)> {
        let mut args = Vec::new();
        let mut cursor = arg_list_node.walk();

        for child in arg_list_node.children(&mut cursor) {
            if child.is_named()
                && let Ok(arg_text) = child.utf8_text(content.as_bytes())
            {
                let arg_name = arg_text.to_string();
                let position = Position {
                    line: child.start_position().row as u32,
                    character: child.start_position().column as u32,
                };
                args.push((arg_name, position));
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
            if child.kind() == "parameter"
                && let Some(type_node) = child.child_by_field_name("type")
                && let Ok(type_name) = type_node.utf8_text(content.as_bytes())
            {
                params.push(type_name.to_string());
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

    /// When `var_name` is an untyped lambda parameter (e.g. the `item` in
    /// `items.forEach { item -> ... }`), returns a `__cp__:…` marker.
    fn find_lambda_param_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(Option<String>, Position)> {
        let mut node = get_node_at_position(tree, content, position)?;

        loop {
            if node.kind() == "lambda_literal" {
                // Check if any lambda_parameter in lambda_parameters matches var_name.
                let mut cursor = node.walk();
                let mut has_explicit_params = false;
                for child in node.children(&mut cursor) {
                    if child.kind() != "lambda_parameters" {
                        continue;
                    }
                    has_explicit_params = true;
                    let mut lambda_param_index = 0usize;
                    let mut pc = child.walk();
                    for param in child.children(&mut pc) {
                        if param.kind() != "variable_declaration" {
                            continue;
                        }
                        let name_node = param.child_by_field_name("name")?;
                        let name = name_node.utf8_text(content.as_bytes()).ok()?;
                        if name == var_name {
                            // Has explicit type annotation?
                            let explicit_type = param
                                .children(&mut param.walk())
                                .find(|n| {
                                    n.kind() == "user_type" || n.kind() == "nullable_type"
                                })
                                .and_then(|t| t.utf8_text(content.as_bytes()).ok())
                                .map(|s| s.to_string());
                            let decl_pos = Position {
                                line: name_node.start_position().row as u32,
                                character: name_node.start_position().column as u32,
                            };
                            if let Some(t) = explicit_type {
                                return Some((Some(t), decl_pos));
                            }
                            let type_str = self.build_lambda_param_marker(
                                &node,
                                content,
                                lambda_param_index,
                            )?;
                            return Some((Some(type_str), decl_pos));
                        }
                        lambda_param_index += 1;
                    }
                }

                // Kotlin implicit `it`: a lambda with no declared parameters uses `it` as
                // the implicit first parameter.
                if !has_explicit_params && var_name == "it" {
                    let type_str = self.build_lambda_param_marker(&node, content, 0)?;
                    let decl_pos = Position {
                        line: node.start_position().row as u32,
                        character: node.start_position().column as u32,
                    };
                    return Some((Some(type_str), decl_pos));
                }
            }
            node = node.parent()?;
        }
    }

    /// Builds a `__cp__:receiver_chain:method_name:method_param_idx:lambda_param_idx`
    /// marker for a lambda parameter at `lambda_param_index` inside `lambda_node`.
    fn build_lambda_param_marker(
        &self,
        lambda_node: &Node,
        content: &str,
        lambda_param_index: usize,
    ) -> Option<String> {
        // Walk up: lambda_literal → annotated_lambda → call_suffix → call_expression
        let annotated = lambda_node.parent()?;
        let call_suffix = annotated.parent()?;
        if call_suffix.kind() != "call_suffix" {
            return None;
        }
        let call_expr = call_suffix.parent()?;
        if call_expr.kind() != "call_expression" {
            return None;
        }

        // call_expression = navigation_expression + call_suffix
        let nav_expr = call_expr.child(0)?;
        if nav_expr.kind() != "navigation_expression" {
            return None;
        }

        // navigation_expression = receiver + navigation_suffix
        let receiver = nav_expr.child(0)?;
        let nav_suffix = nav_expr.child(1)?;
        let method_name_node = nav_suffix.named_child(0)?;
        let method_name = method_name_node.utf8_text(content.as_bytes()).ok()?;
        let receiver_chain = Self::extract_invocation_chain(&receiver, content)?;

        // The lambda is effectively the last argument (method_param_idx = number of
        // value_arguments in the call_suffix before the lambda).
        let value_args_count = call_suffix
            .children(&mut call_suffix.walk())
            .find(|n| n.kind() == "value_arguments")
            .map(|va| va.named_child_count())
            .unwrap_or(0);

        Some(format!(
            "__cp__:{}:{}:{}:{}",
            receiver_chain, method_name, value_args_count, lambda_param_index
        ))
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
            Some(_) => ts_helper::get_many(node, source, &GET_MODIFIERS_QUERY, Some(1)),
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
        match self.get_kind(node) {
            Some(NodeKind::Function) | Some(NodeKind::Class) => {
                let params = ts_helper::get_many(node, source, &GET_PARAMETERS_QUERY, Some(1))
                    .into_iter()
                    .map(|p| self.parse_parameter(&p))
                    .collect();
                Some(params)
            }
            _ => None,
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

    fn get_implicit_imports(&self) -> Vec<String> {
        KOTLIN_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        let mut result = None;
        let mut cursor = QueryCursor::new();
        cursor
            .matches(&GET_TYPE_QUERY, node, content.as_bytes())
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
            current = current.parent()?;
        }
    }

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String> {
        let point = Point::new(position.line as usize, position.character as usize);
        let mut node = tree.root_node().descendant_for_point_range(point, point)?;

        loop {
            match node.kind() {
                "collection_literal" => return Some("List".to_string()),

                "decimal_integer_literal" | "hex_literal" | "bin_literal" => {
                    if node.utf8_text(content.as_bytes()).is_ok() {
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

            node = node.parent()?;
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

        let reference_byte = ts_helper::position_to_byte_offset(content, position);
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

        // var_name was not found as a regular local variable — check if it is an
        // untyped lambda parameter inside an enclosing lambda_literal.
        self.find_lambda_param_declaration(tree, content, var_name, position)
    }

    fn find_declarations_in_scope(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Vec<(String, Option<String>)> {
        let Some(mut current_node) = get_node_at_position(tree, content, position) else {
            return vec![];
        };
        let reference_byte = ts_helper::position_to_byte_offset(content, position);
        let mut results = Vec::new();
        loop {
            self.collect_declarations_in_scope(current_node, content, reference_byte, &mut results);
            if let Some(parent) = current_node.parent() {
                current_node = parent;
            } else {
                break;
            }
        }
        results
    }

    fn collect_diagnostics(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<tower_lsp::lsp_types::Diagnostic> {
        let mut diagnostics = Vec::new();
        collect_syntax_errors(tree.root_node(), source, &mut diagnostics);
        diagnostics
    }
}

#[allow(dead_code)]
mod tests {
    use tower_lsp::lsp_types::Position;
    use tree_sitter::Node;

    mod extract_call_arguments;
    mod find_declarations_in_scope;
    mod find_ident_at_position;
    mod find_variable_type;
    mod get_imports;
    mod get_indexer_data;
    mod get_literal_type;
    mod get_method_receiver_and_params;
    mod get_type_at_position;

    fn find_position(content: &str, marker: &str) -> Position {
        content
            .lines()
            .enumerate()
            .find_map(|(line_num, line)| {
                line.find(marker)
                    .map(|col| Position::new(line_num as u32, col as u32))
            })
            .unwrap_or_else(|| panic!("Marker '{}' not found", marker))
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
