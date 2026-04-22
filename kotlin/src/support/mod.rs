use lsp_core::{
    language_support::{CallArgData, ClassDeclarationData, GenericTypeUsage, IdentResult, LanguageSupport, MemberAccessData, MethodCallSiteData, OverrideMethodData, ParameterResult, ParseResult},
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, collect_syntax_errors, get_node_at_position, node_contains_position},
};
use std::{cell::RefCell, collections::HashSet, fs, path::Path, sync::LazyLock};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::{
    constants::KOTLIN_IMPLICIT_IMPORTS,
    support::queries::{
        CLASS_METHOD_NAMES_QUERY, DECLARED_TYPES_QUERY, DECLARES_VARIABLE_QUERY,
        FUNCTION_WITH_RETURN_QUERY, GET_ANNOTATIONS_QUERY, GET_EXTENDS_QUERY,
        GET_FIELD_RETURN_QUERY, GET_FIELD_SHORT_NAME_QUERY, GET_FUNCTION_RETURN_QUERY,
        GET_GENERIC_TYPE_USAGES_QUERY, GET_IMPLEMENTS_QUERY, GET_IMPORTS_QUERY, GET_KDOC_QUERY,
        GET_MEMBER_ACCESSES_QUERY, GET_METHOD_CALL_SITES_QUERY, GET_MODIFIERS_QUERY, GET_OVERRIDE_METHODS_QUERY,
        GET_PACKAGE_NAME_QUERY, GET_PARAMETERS_QUERY, GET_SHORT_NAME_QUERY, GET_TYPE_QUERY,
        GET_TYPE_REFS_QUERY, IDENT_QUERY,
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

/// Returns true if `node` contains a return or throw `jump_expression` without
/// crossing into inner class or lambda boundaries.
fn has_return_in_block(node: tree_sitter::Node, bytes: &[u8]) -> bool {
    if node.kind() == "jump_expression" {
        let text = node.utf8_text(bytes).unwrap_or("");
        return text.starts_with("return") || text.starts_with("throw");
    }
    if matches!(
        node.kind(),
        "class_declaration" | "object_declaration" | "lambda_literal"
    ) {
        return false;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if has_return_in_block(child, bytes) {
            return true;
        }
    }
    false
}

fn collect_missing_returns(
    tree: &Tree,
    source: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();
    let ret_type_idx = FUNCTION_WITH_RETURN_QUERY.capture_index_for_name("ret_type");
    let name_idx = FUNCTION_WITH_RETURN_QUERY.capture_index_for_name("name");

    cursor
        .matches(&FUNCTION_WITH_RETURN_QUERY, tree.root_node(), bytes)
        .for_each(|m| {
            let Some(ret_cap) = m.captures.iter().find(|c| Some(c.index) == ret_type_idx) else {
                return;
            };
            let Some(name_cap) = m.captures.iter().find(|c| Some(c.index) == name_idx) else {
                return;
            };

            // skip Unit and Nothing (always throws)
            let ret_text = ret_cap.node.utf8_text(bytes).unwrap_or("").trim();
            if ret_text == "Unit" || ret_text == "Nothing" {
                return;
            }

            // parent of the name node is the function_declaration
            let Some(func_node) = name_cap.node.parent() else {
                return;
            };

            // find function_body; then check if it is a block body (has statements child)
            // vs expression body (direct expression) — expression body always returns
            let mut func_body_opt = None;
            let mut c = func_node.walk();
            for child in func_node.children(&mut c) {
                if child.kind() == "function_body" {
                    func_body_opt = Some(child);
                    break;
                }
            }
            let Some(func_body) = func_body_opt else {
                return; // abstract or interface function — no body
            };

            // Determine block body vs expression body by checking for a statements child
            let is_block_body = {
                let mut c2 = func_body.walk();
                func_body
                    .children(&mut c2)
                    .any(|ch| ch.kind() == "statements")
            };
            if !is_block_body {
                return; // expression body always implicitly returns
            }

            if !has_return_in_block(func_body, bytes) {
                let range = node_to_range(&name_cap.node);
                let name = name_cap.node.utf8_text(bytes).unwrap_or("?");
                diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                    range,
                    severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "missing_return_statement".to_string(),
                    )),
                    source: Some("lspintar".to_string()),
                    message: format!("Missing return statement in function '{name}'"),
                    ..Default::default()
                });
            }
        });
}

fn node_to_range(node: &tree_sitter::Node) -> Range {
    Range {
        start: tower_lsp::lsp_types::Position {
            line: node.start_position().row as u32,
            character: node.start_position().column as u32,
        },
        end: tower_lsp::lsp_types::Position {
            line: node.end_position().row as u32,
            character: node.end_position().column as u32,
        },
    }
}

fn collect_duplicate_imports(
    tree: &Tree,
    source: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let mut cursor = QueryCursor::new();
    let bytes = source.as_bytes();
    let mut seen: std::collections::HashMap<String, Range> = std::collections::HashMap::new();

    cursor
        .matches(&GET_IMPORTS_QUERY, tree.root_node(), bytes)
        .for_each(|m| {
            let Some(cap) = m.captures.first() else {
                return;
            };
            let node = cap.node;
            let Ok(text) = node.utf8_text(bytes) else {
                return;
            };
            // Kotlin: "import foo.bar.Baz" or "import foo.bar.Baz as Alias" (no semicolon)
            let fqn = text
                .trim_start_matches("import ")
                .trim()
                .to_string();
            let range = node_to_range(&node);
            if seen.contains_key(&fqn) {
                diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                    range,
                    severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "duplicate_import".to_string(),
                    )),
                    source: Some("lspintar".to_string()),
                    message: format!("Duplicate import: {fqn}"),
                    ..Default::default()
                });
            } else {
                seen.insert(fqn, range);
            }
        });
}

fn collect_unused_imports(
    tree: &Tree,
    source: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let mut cursor = QueryCursor::new();
    let bytes = source.as_bytes();
    let mut imports: Vec<(String, String, Range)> = Vec::new();

    cursor
        .matches(&GET_IMPORTS_QUERY, tree.root_node(), bytes)
        .for_each(|m| {
            let Some(cap) = m.captures.first() else {
                return;
            };
            let node = cap.node;
            let Ok(text) = node.utf8_text(bytes) else {
                return;
            };
            let raw = text.trim_start_matches("import ").trim();
            if raw.ends_with(".*") {
                return;
            }
            // Handle "import X as Y" — the alias is what appears in the body.
            let (fqn, simple) = if let Some((base, alias)) = raw.split_once(" as ") {
                (base.trim().to_string(), alias.trim().to_string())
            } else {
                let s = raw.split('.').next_back().unwrap_or(raw).to_string();
                (raw.to_string(), s)
            };
            imports.push((fqn, simple, node_to_range(&node)));
        });

    if imports.is_empty() {
        return;
    }

    let import_section_end = imports
        .iter()
        .map(|(_, _, r)| {
            let line = r.end.line as usize;
            source
                .lines()
                .take(line + 1)
                .map(|l| l.len() + 1)
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let body = if import_section_end < source.len() {
        &source[import_section_end..]
    } else {
        ""
    };

    for (fqn, simple, range) in imports {
        if !body_uses_name(body, &simple) {
            diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                range,
                severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "unused_import".to_string(),
                )),
                source: Some("lspintar".to_string()),
                message: format!("Unused import: {fqn}"),
                ..Default::default()
            });
        }
    }
}

/// Returns true if `name` appears as a word-boundary token in `body`.
fn body_uses_name(body: &str, name: &str) -> bool {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    let mut start = 0;
    while let Some(pos) = body[start..].find(name) {
        let abs = start + pos;
        let before_ok = abs == 0 || !is_id(body[..abs].chars().next_back().unwrap());
        let after_ok = abs + name.len() >= body.len()
            || !is_id(body[abs + name.len()..].chars().next().unwrap());
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}

fn collect_unchecked_casts(
    tree: &Tree,
    source: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let bytes = source.as_bytes();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "as_expression" {
            // Find the user_type child (the cast target)
            let mut type_node_opt = None;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "user_type" {
                    type_node_opt = Some(child);
                }
            }
            if let Some(type_node) = type_node_opt {
                let has_type_args = {
                    let mut c = type_node.walk();
                    type_node.children(&mut c).any(|ch| ch.kind() == "type_arguments")
                };
                if has_type_args {
                    let type_text = type_node.utf8_text(bytes).unwrap_or("?");
                    diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                        range: node_to_range(&type_node),
                        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING),
                        code: Some(tower_lsp::lsp_types::NumberOrString::String(
                            "unchecked_cast".to_string(),
                        )),
                        source: Some("lspintar".to_string()),
                        message: format!(
                            "Unchecked cast to '{type_text}'; type arguments cannot be verified at runtime due to type erasure"
                        ),
                        ..Default::default()
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn extract_param_types(func_node: tree_sitter::Node, bytes: &[u8]) -> Vec<String> {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "parameters" {
            let mut param_types = Vec::new();
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                if param.kind() == "parameter" {
                    if let Some(type_node) = param.child_by_field_name("type") {
                        if let Ok(t) = type_node.utf8_text(bytes) {
                            param_types.push(t.to_string());
                        }
                    }
                }
            }
            return param_types;
        }
    }
    Vec::new()
}

fn check_body_for_dup_sigs(
    body_node: tree_sitter::Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let mut seen: std::collections::HashMap<String, ()> = std::collections::HashMap::new();
    let mut cursor = body_node.walk();
    for child in body_node.children(&mut cursor) {
        if child.kind() != "function_declaration" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else { continue; };
        let Ok(name) = name_node.utf8_text(bytes) else { continue; };
        let param_types = extract_param_types(child, bytes);
        let sig = format!("{}({})", name, param_types.join(","));
        if seen.contains_key(&sig) {
            diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                range: node_to_range(&name_node),
                severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "duplicate_method_signature".to_string(),
                )),
                source: Some("lspintar".to_string()),
                message: format!("Duplicate method signature: '{sig}'"),
                ..Default::default()
            });
        } else {
            seen.insert(sig, ());
        }
    }
}

fn collect_duplicate_method_signatures(
    tree: &Tree,
    source: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let bytes = source.as_bytes();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if kind == "class_body" || kind == "enum_class_body" {
            check_body_for_dup_sigs(node, bytes, diagnostics);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
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
        collect_duplicate_imports(tree, source, &mut diagnostics);
        collect_unused_imports(tree, source, &mut diagnostics);
        collect_missing_returns(tree, source, &mut diagnostics);
        collect_duplicate_method_signatures(tree, source, &mut diagnostics);
        collect_unchecked_casts(tree, source, &mut diagnostics);
        collect_variable_used_before_assignment(tree.root_node(), source.as_bytes(), &mut diagnostics);
        collect_literal_type_mismatches(tree.root_node(), source.as_bytes(), &mut diagnostics);
        collect_null_safety_violations(tree.root_node(), source.as_bytes(), &mut diagnostics);
        diagnostics
    }

    fn get_type_references(&self, tree: &Tree, source: &str) -> Vec<(String, Range)> {
        let mut cursor = QueryCursor::new();
        let bytes = source.as_bytes();
        let mut refs = Vec::new();

        cursor
            .matches(&GET_TYPE_REFS_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                for cap in m.captures {
                    let node = cap.node;
                    let Ok(text) = node.utf8_text(bytes) else {
                        return;
                    };
                    refs.push((text.to_string(), node_to_range(&node)));
                }
            });

        refs
    }

    fn get_declared_type_names(&self, tree: &Tree, source: &str) -> Vec<String> {
        let mut cursor = QueryCursor::new();
        let bytes = source.as_bytes();
        let mut names = Vec::new();

        cursor
            .matches(&DECLARED_TYPES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                if let Some(cap) = m.captures.first() {
                    if let Ok(text) = cap.node.utf8_text(bytes) {
                        names.push(text.to_string());
                    }
                }
            });

        names
    }

    fn get_class_declarations(&self, tree: &Tree, source: &str) -> Vec<ClassDeclarationData> {
        let bytes = source.as_bytes();
        let mut results = Vec::new();
        let mut cursor = QueryCursor::new();

        cursor
            .matches(&DECLARED_TYPES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(name_cap) = m.captures.first() else { return; };
                let name_node = name_cap.node;
                let Some(type_node) = name_node.parent() else { return; };

                // Only class and object declarations can have unimplemented methods.
                // interface_declaration is excluded (same as Java).
                let kind = type_node.kind();
                if kind != "class_declaration" && kind != "object_declaration" {
                    return;
                }

                let Ok(name) = name_node.utf8_text(bytes) else { return; };
                let ident_range = node_to_range(&name_node);
                let modifiers = self.get_modifiers(&type_node, source);
                let is_abstract = modifiers.iter().any(|m| m == "abstract");

                let extends: Vec<String> = self.get_extends(&type_node, source).into_iter().collect();
                let implements = self.get_implements(&type_node, source);
                let mut parents = extends;
                parents.extend(implements);

                let mut defined_method_names = std::collections::HashSet::new();
                for i in 0..type_node.child_count() {
                    let Some(child) = type_node.child(i) else { continue };
                    if child.kind() == "class_body" || child.kind() == "enum_class_body" {
                        let mut method_cursor = QueryCursor::new();
                        method_cursor.set_max_start_depth(Some(1));
                        method_cursor
                            .matches(&CLASS_METHOD_NAMES_QUERY, child, bytes)
                            .for_each(|mm| {
                                if let Some(cap) = mm.captures.first() {
                                    if let Ok(method_name) = cap.node.utf8_text(bytes) {
                                        defined_method_names.insert(method_name.to_string());
                                    }
                                }
                            });
                        break;
                    }
                }

                results.push(ClassDeclarationData {
                    name: name.to_string(),
                    ident_range,
                    is_abstract,
                    parents,
                    defined_method_names,
                });
            });

        results
    }

    fn get_member_accesses(&self, tree: &Tree, source: &str) -> Vec<MemberAccessData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let recv_idx = GET_MEMBER_ACCESSES_QUERY.capture_index_for_name("receiver");
        let meth_idx = GET_MEMBER_ACCESSES_QUERY.capture_index_for_name("method");

        cursor
            .matches(&GET_MEMBER_ACCESSES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(recv_cap) = m.captures.iter().find(|c| Some(c.index) == recv_idx) else {
                    return;
                };
                let Some(meth_cap) = m.captures.iter().find(|c| Some(c.index) == meth_idx) else {
                    return;
                };
                let Ok(receiver_name) = recv_cap.node.utf8_text(bytes) else { return; };
                let Ok(member_name) = meth_cap.node.utf8_text(bytes) else { return; };
                results.push(MemberAccessData {
                    receiver_name: receiver_name.to_string(),
                    member_name: member_name.to_string(),
                    member_range: node_to_range(&meth_cap.node),
                    receiver_range: node_to_range(&recv_cap.node),
                });
            });

        results
    }

    fn get_generic_type_usages(&self, tree: &Tree, source: &str) -> Vec<GenericTypeUsage> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let base_idx = GET_GENERIC_TYPE_USAGES_QUERY.capture_index_for_name("base");
        let args_idx = GET_GENERIC_TYPE_USAGES_QUERY.capture_index_for_name("args");

        cursor
            .matches(&GET_GENERIC_TYPE_USAGES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(base_cap) = m.captures.iter().find(|c| Some(c.index) == base_idx) else {
                    return;
                };
                let Some(args_cap) = m.captures.iter().find(|c| Some(c.index) == args_idx) else {
                    return;
                };
                let Ok(type_name) = base_cap.node.utf8_text(bytes) else { return; };
                // In Kotlin, type_arguments contains type_projection nodes (and commas).
                let arg_count = args_cap
                    .node
                    .named_children(&mut args_cap.node.walk())
                    .filter(|n| n.kind() == "type_projection")
                    .count();
                let Some(user_type_node) = base_cap.node.parent() else { return; };
                results.push(GenericTypeUsage {
                    type_name: type_name.to_string(),
                    arg_count,
                    range: node_to_range(&user_type_node),
                });
            });

        results
    }

    fn get_override_methods(&self, tree: &Tree, source: &str) -> Vec<OverrideMethodData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let mod_idx = GET_OVERRIDE_METHODS_QUERY.capture_index_for_name("mod");
        let name_idx = GET_OVERRIDE_METHODS_QUERY.capture_index_for_name("name");

        cursor
            .matches(&GET_OVERRIDE_METHODS_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(mod_cap) = m.captures.iter().find(|c| Some(c.index) == mod_idx) else {
                    return;
                };
                let Ok(mod_text) = mod_cap.node.utf8_text(bytes) else { return };
                if mod_text != "override" {
                    return;
                }
                let Some(name_cap) = m.captures.iter().find(|c| Some(c.index) == name_idx) else {
                    return;
                };
                let Ok(method_name) = name_cap.node.utf8_text(bytes) else { return };
                // Return type is an optional field on function_declaration — walk up to get it.
                let return_type = name_cap
                    .node
                    .parent()
                    .and_then(|func| func.child_by_field_name("return_type"))
                    .and_then(|rt| rt.utf8_text(bytes).ok())
                    .filter(|s| *s != "Unit")
                    .map(|s| s.to_string());
                let Some(containing_class) = find_containing_class(name_cap.node, bytes) else {
                    return;
                };
                results.push(OverrideMethodData {
                    containing_class,
                    method_name: method_name.to_string(),
                    return_type,
                    range: node_to_range(&name_cap.node),
                });
            });

        results
    }

    fn get_method_call_sites(&self, tree: &Tree, source: &str) -> Vec<MethodCallSiteData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let recv_idx = GET_METHOD_CALL_SITES_QUERY.capture_index_for_name("receiver");
        let meth_idx = GET_METHOD_CALL_SITES_QUERY.capture_index_for_name("method");
        let args_idx = GET_METHOD_CALL_SITES_QUERY.capture_index_for_name("args");

        cursor
            .matches(&GET_METHOD_CALL_SITES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(recv_cap) = m.captures.iter().find(|c| Some(c.index) == recv_idx) else {
                    return;
                };
                let Some(meth_cap) = m.captures.iter().find(|c| Some(c.index) == meth_idx) else {
                    return;
                };
                let Some(args_cap) = m.captures.iter().find(|c| Some(c.index) == args_idx) else {
                    return;
                };
                let Ok(receiver_name) = recv_cap.node.utf8_text(bytes) else { return };
                let Ok(method_name) = meth_cap.node.utf8_text(bytes) else { return };

                // In Kotlin, value_arguments contains value_argument children, each wrapping
                // the actual expression.
                let mut args = Vec::new();
                let mut arg_cursor = args_cap.node.walk();
                for va in args_cap.node.children(&mut arg_cursor) {
                    if va.kind() != "value_argument" {
                        continue;
                    }
                    // The actual expression is the first named child of value_argument
                    // (skipping optional named-argument label).
                    let mut va_cursor = va.walk();
                    let expr = va.children(&mut va_cursor).find(|n| n.is_named());
                    let Some(expr) = expr else { continue };
                    let node_kind = expr.kind().to_string();
                    let text = expr.utf8_text(bytes).unwrap_or("").to_string();
                    args.push(CallArgData {
                        node_kind,
                        text,
                        range: node_to_range(&expr),
                    });
                }

                results.push(MethodCallSiteData {
                    receiver_name: receiver_name.to_string(),
                    receiver_range: node_to_range(&recv_cap.node),
                    method_name: method_name.to_string(),
                    method_range: node_to_range(&meth_cap.node),
                    args,
                });
            });

        results
    }

    fn reserved_keywords(&self) -> &'static HashSet<&'static str> {
        &KOTLIN_KEYWORDS
    }

    fn find_local_references(
        &self,
        tree: &Tree,
        content: &str,
        decl_position: &Position,
    ) -> Option<Vec<Range>> {
        lsp_core::local_refs::find_local_references(
            tree,
            content,
            decl_position,
            KOTLIN_DECL_NODE_KINDS,
            KOTLIN_SCOPE_NODE_KINDS,
        )
    }
}

static KOTLIN_KEYWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // Hard keywords — always reserved.
        "as", "break", "class", "continue", "do", "else", "false", "for", "fun",
        "if", "in", "interface", "is", "null", "object", "package", "return",
        "super", "this", "throw", "true", "try", "typealias", "typeof", "val",
        "var", "when", "while",
        // Modifier keywords commonly treated as reserved in identifier contexts
        // by IntelliJ; conservative to reject them.
        "abstract", "actual", "annotation", "by", "companion", "const",
        "crossinline", "data", "enum", "expect", "external", "final", "infix",
        "init", "inline", "inner", "internal", "lateinit", "noinline", "open",
        "operator", "out", "override", "private", "protected", "public",
        "reified", "sealed", "suspend", "tailrec", "vararg",
    ]
    .into_iter()
    .collect()
});

static KOTLIN_DECL_NODE_KINDS: &[&str] = &[
    "property_declaration",
    "variable_declaration",
    "multi_variable_declaration",
    "parameter",
    "function_value_parameter",
    "lambda_parameters",
    "class_parameter",
    "for_statement",
];

static KOTLIN_SCOPE_NODE_KINDS: &[&str] = &[
    "function_declaration",
    "secondary_constructor",
    "primary_constructor",
    "lambda_literal",
    "anonymous_function",
    "function_body",
    "statements",
    "class_body",
    "block",
    "for_statement",
    "when_expression",
    "catch_block",
    "if_expression",
];

fn find_containing_class(mut node: Node, bytes: &[u8]) -> Option<String> {
    while let Some(parent) = node.parent() {
        if parent.kind() == "class_declaration" {
            let mut walker = parent.walk();
            for child in parent.children(&mut walker) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    return child.utf8_text(bytes).ok().map(|s| s.to_string());
                }
            }
        }
        node = parent;
    }
    None
}

/// Walks the AST looking for `statements` nodes (Kotlin function/lambda bodies) and, within
/// each, linearly tracks variables declared without an initializer (`val x: T` / `var x: T`),
/// flagging any read that occurs before the first plain assignment to that variable.
fn collect_variable_used_before_assignment(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if node.kind() == "statements" {
        let mut uninit: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut cursor = node.walk();
        for stmt in node.children(&mut cursor) {
            kotlin_process_statements_stmt(stmt, bytes, &mut uninit, diagnostics);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_variable_used_before_assignment(child, bytes, diagnostics);
    }
}

fn kotlin_process_statements_stmt(
    stmt: Node,
    bytes: &[u8],
    uninit: &mut std::collections::HashSet<String>,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    match stmt.kind() {
        "property_declaration" => {
            let has_value = stmt.child_by_field_name("value").is_some();
            // Name lives at: property_declaration → variable_declaration → name: identifier
            let mut var_name: Option<String> = None;
            {
                let mut c = stmt.walk();
                for child in stmt.children(&mut c) {
                    if child.kind() == "variable_declaration" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                var_name = Some(name.to_string());
                            }
                        }
                        break;
                    }
                }
            }
            if let Some(name) = var_name {
                if has_value {
                    if let Some(val) = stmt.child_by_field_name("value") {
                        kotlin_scan_reads(val, bytes, uninit, diagnostics);
                    }
                    uninit.remove(&name);
                } else {
                    uninit.insert(name);
                }
            }
        }
        "assignment" => {
            // child(0) = directly_assignable_expression, child(1) = operator, child(2) = RHS
            let is_pure = stmt
                .child(1)
                .and_then(|op| op.utf8_text(bytes).ok())
                .map(|op| op == "=")
                .unwrap_or(false);
            if is_pure {
                let lhs_name = stmt
                    .child(0)
                    .filter(|n| n.kind() == "directly_assignable_expression")
                    .and_then(|dae| dae.named_child(0))
                    .filter(|n| n.kind() == "identifier")
                    .and_then(|n| n.utf8_text(bytes).ok())
                    .map(|s| s.to_string());
                if let Some(name) = lhs_name {
                    if uninit.contains(&name) {
                        if let Some(rhs) = stmt.child(2) {
                            kotlin_scan_reads(rhs, bytes, uninit, diagnostics);
                        }
                        uninit.remove(&name);
                        return;
                    }
                }
            }
            kotlin_scan_reads(stmt, bytes, uninit, diagnostics);
        }
        _ => {
            collect_variable_used_before_assignment(stmt, bytes, diagnostics);
        }
    }
}

fn kotlin_scan_reads(
    node: Node,
    bytes: &[u8],
    uninit: &std::collections::HashSet<String>,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if uninit.is_empty() {
        return;
    }
    match node.kind() {
        "assignment" => {
            let is_pure = node
                .child(1)
                .and_then(|op| op.utf8_text(bytes).ok())
                .map(|op| op == "=")
                .unwrap_or(false);
            if !is_pure {
                if let Some(lhs) = node.child(0) {
                    kotlin_scan_reads(lhs, bytes, uninit, diagnostics);
                }
            }
            if let Some(rhs) = node.child(2) {
                kotlin_scan_reads(rhs, bytes, uninit, diagnostics);
            }
        }
        "identifier" => {
            if let Ok(name) = node.utf8_text(bytes) {
                if uninit.contains(name) {
                    diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                        range: node_to_range(&node),
                        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                        code: Some(tower_lsp::lsp_types::NumberOrString::String(
                            "variable_used_before_assignment".to_string(),
                        )),
                        source: Some("lspintar".to_string()),
                        message: format!("Variable '{}' may not have been initialized", name),
                        ..Default::default()
                    });
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                kotlin_scan_reads(child, bytes, uninit, diagnostics);
            }
        }
    }
}

/// Walks `source_file` and `statements` blocks sequentially, tracking which variables
/// are declared with nullable types.  Flags:
///   - null literal assigned to a non-nullable declared type
///   - null literal returned from a function with a non-nullable return type
///   - nullable identifier assigned bare to a non-nullable declared type (no `!!`, `?:`, etc.)
fn collect_null_safety_violations(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if matches!(node.kind(), "source_file" | "statements") {
        let mut nullable: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut cursor = node.walk();
        for stmt in node.children(&mut cursor) {
            kotlin_null_stmt(stmt, bytes, &mut nullable, diagnostics);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_null_safety_violations(child, bytes, diagnostics);
    }
}

fn kotlin_null_stmt(
    stmt: Node,
    bytes: &[u8],
    nullable: &mut std::collections::HashSet<String>,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    match stmt.kind() {
        "property_declaration" => {
            let mut cursor = stmt.walk();
            let var_decl = stmt
                .children(&mut cursor)
                .find(|n| n.kind() == "variable_declaration");
            let Some(var_decl) = var_decl else { return };

            let type_node = var_decl.child_by_field_name("type");
            let value = stmt.child_by_field_name("value");

            let is_nullable_decl = type_node
                .map(|n| n.kind() == "nullable_type")
                .unwrap_or(false);

            if is_nullable_decl {
                if let Some(name_node) = var_decl.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(bytes) {
                        nullable.insert(name.to_string());
                    }
                }
            } else if let Some(type_n) = type_node {
                if let Ok(type_text) = type_n.utf8_text(bytes) {
                    if let Some(val) = value {
                        let val_kind = val.kind();
                        if val_kind == "null_literal" {
                            diagnostics.push(make_null_safety_diag(&val, type_text));
                        } else if val_kind == "identifier" {
                            let val_name = val.utf8_text(bytes).unwrap_or("");
                            if nullable.contains(val_name) {
                                diagnostics.push(make_null_safety_diag(&val, type_text));
                            }
                        }
                    }
                }
            }
        }
        "function_declaration" => {
            // Check null returns inside this function
            kotlin_check_null_returns(stmt, bytes, diagnostics);
            // Recurse into the body with a fresh nullable scope
            if let Some(body) = stmt.child_by_field_name("body") {
                collect_null_safety_violations(body, bytes, diagnostics);
            }
        }
        "jump_expression" => {
            kotlin_check_null_jump_expr(stmt, bytes, diagnostics);
        }
        _ => {
            collect_null_safety_violations(stmt, bytes, diagnostics);
        }
    }
}

fn kotlin_check_null_jump_expr(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let first = node.child(0);
    if first.and_then(|n| n.utf8_text(bytes).ok()).as_deref() != Some("return") {
        return;
    }
    let mut cursor = node.walk();
    let value = node.children(&mut cursor).find(|n| n.is_named());
    let Some(value) = value else { return };
    if value.kind() != "null_literal" {
        return;
    }
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "lambda_literal" | "anonymous_initializer" => return,
            "function_declaration" => {
                let Some(ret_node) = p.child_by_field_name("return_type") else { return };
                if ret_node.kind() == "nullable_type" {
                    return;
                }
                let Ok(ret_text) = ret_node.utf8_text(bytes) else { return };
                if ret_text == "Unit" {
                    return;
                }
                diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                    range: node_to_range(&value),
                    severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "null_safety_violation".to_string(),
                    )),
                    source: Some("lspintar".to_string()),
                    message: format!(
                        "Null cannot be a value of a non-null type '{ret_text}'"
                    ),
                    ..Default::default()
                });
                return;
            }
            _ => {}
        }
        parent = p.parent();
    }
}

/// Walk a function's body looking for `jump_expression` returns with null, used when
/// the function body is not yet entered via the sequential scan (e.g. nested functions).
fn kotlin_check_null_returns(
    func_node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let Some(ret_node) = func_node.child_by_field_name("return_type") else { return };
    if ret_node.kind() == "nullable_type" {
        return;
    }
    let Ok(ret_text) = ret_node.utf8_text(bytes) else { return };
    if ret_text == "Unit" {
        return;
    }
    // Walk the body looking for jump_expressions with null (don't cross lambda boundaries)
    kotlin_find_null_returns(func_node, bytes, ret_text, diagnostics);
}

fn kotlin_find_null_returns<'a>(
    node: Node<'a>,
    bytes: &[u8],
    ret_text: &str,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if matches!(node.kind(), "lambda_literal" | "anonymous_initializer") {
        return;
    }
    if node.kind() == "jump_expression" {
        let first = node.child(0);
        if first.and_then(|n| n.utf8_text(bytes).ok()).as_deref() == Some("return") {
            let mut cursor = node.walk();
            if let Some(val) = node.children(&mut cursor).find(|n| n.is_named()) {
                if val.kind() == "null_literal" {
                    diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                        range: node_to_range(&val),
                        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                        code: Some(tower_lsp::lsp_types::NumberOrString::String(
                            "null_safety_violation".to_string(),
                        )),
                        source: Some("lspintar".to_string()),
                        message: format!(
                            "Null cannot be a value of a non-null type '{ret_text}'"
                        ),
                        ..Default::default()
                    });
                }
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        kotlin_find_null_returns(child, bytes, ret_text, diagnostics);
    }
}

fn make_null_safety_diag(
    node: &Node,
    expected_type: &str,
) -> tower_lsp::lsp_types::Diagnostic {
    tower_lsp::lsp_types::Diagnostic {
        range: node_to_range(node),
        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            "null_safety_violation".to_string(),
        )),
        source: Some("lspintar".to_string()),
        message: format!("Null cannot be a value of a non-null type '{expected_type}'"),
        ..Default::default()
    }
}

fn kotlin_is_literal_type_mismatch(type_text: &str, value_kind: &str) -> bool {
    let is_int_lit = matches!(value_kind, "decimal_integer_literal" | "hex_literal" | "bin_literal");
    let is_float_lit = value_kind == "real_literal";
    let is_bool_lit = value_kind == "boolean_literal";
    let is_string_lit = matches!(value_kind, "string_literal" | "multiline_string_literal");
    let is_char_lit = value_kind == "character_literal";

    match type_text {
        "Int" | "Long" | "Short" | "Byte" => is_bool_lit || is_string_lit || is_float_lit,
        "Float" | "Double" => is_bool_lit || is_string_lit,
        "Boolean" => is_int_lit || is_float_lit || is_string_lit || is_char_lit,
        "String" => is_int_lit || is_float_lit || is_bool_lit || is_char_lit,
        "Char" => is_bool_lit || is_string_lit || is_float_lit,
        _ => false,
    }
}

fn kotlin_literal_type_name(value_kind: &str, value_text: &str) -> &'static str {
    match value_kind {
        "decimal_integer_literal" | "hex_literal" | "bin_literal" => {
            if value_text.ends_with('L') { "Long" } else { "Int" }
        }
        "real_literal" => {
            if value_text.ends_with('f') || value_text.ends_with('F') { "Float" } else { "Double" }
        }
        "boolean_literal" => "Boolean",
        "string_literal" | "multiline_string_literal" => "String",
        "character_literal" => "Char",
        _ => "unknown",
    }
}

fn collect_literal_type_mismatches(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    match node.kind() {
        "property_declaration" => {
            check_kotlin_property_literal(node, bytes, diagnostics);
        }
        "jump_expression" => {
            check_kotlin_return_literal(node, bytes, diagnostics);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_literal_type_mismatches(child, bytes, diagnostics);
    }
}

fn check_kotlin_property_literal(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let Some(value) = node.child_by_field_name("value") else { return };
    let value_kind = value.kind();
    let value_text = value.utf8_text(bytes).unwrap_or("");

    let mut cursor = node.walk();
    let var_decl = node.children(&mut cursor).find(|n| n.kind() == "variable_declaration");
    let Some(var_decl) = var_decl else { return };
    let Some(type_node) = var_decl.child_by_field_name("type") else { return };

    // nullable types (Int?) — assignment of null is valid; skip
    if type_node.kind() == "nullable_type" {
        return;
    }
    let Ok(type_text) = type_node.utf8_text(bytes) else { return };

    if kotlin_is_literal_type_mismatch(type_text, value_kind) {
        let inferred = kotlin_literal_type_name(value_kind, value_text);
        diagnostics.push(tower_lsp::lsp_types::Diagnostic {
            range: node_to_range(&value),
            severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "type_mismatch_assignment".to_string(),
            )),
            source: Some("lspintar".to_string()),
            message: format!("Type mismatch: cannot convert from '{inferred}' to '{type_text}'"),
            ..Default::default()
        });
    }
}

fn check_kotlin_return_literal(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    // "return" is an unnamed first child; skip break/continue
    let first = node.child(0);
    if first.and_then(|n| n.utf8_text(bytes).ok()).as_deref() != Some("return") {
        return;
    }

    // The return value is the first named child after "return"
    let mut cursor = node.walk();
    let value = node.children(&mut cursor).find(|n| n.is_named());
    let Some(value) = value else { return };
    let value_kind = value.kind();
    let value_text = value.utf8_text(bytes).unwrap_or("");

    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "lambda_literal" | "anonymous_initializer" => return,
            "function_declaration" => {
                let Some(ret_node) = p.child_by_field_name("return_type") else { return };
                if ret_node.kind() == "nullable_type" {
                    return;
                }
                let Ok(ret_text) = ret_node.utf8_text(bytes) else { return };
                if ret_text == "Unit" {
                    return;
                }
                if kotlin_is_literal_type_mismatch(ret_text, value_kind) {
                    let inferred = kotlin_literal_type_name(value_kind, value_text);
                    diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                        range: node_to_range(&value),
                        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                        code: Some(tower_lsp::lsp_types::NumberOrString::String(
                            "incompatible_return_type".to_string(),
                        )),
                        source: Some("lspintar".to_string()),
                        message: format!(
                            "Return type mismatch: cannot convert from '{inferred}' to '{ret_text}'"
                        ),
                        ..Default::default()
                    });
                }
                return;
            }
            _ => {}
        }
        parent = p.parent();
    }
}

#[allow(dead_code)]
mod tests {
    use tower_lsp::lsp_types::Position;
    use tree_sitter::Node;

    mod collect_diagnostics;
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
