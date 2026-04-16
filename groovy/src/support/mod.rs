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
    constants::GROOVY_IMPLICIT_IMPORTS,
    support::queries::{
        DECLARES_VARIABLE_QUERY, GET_ANNOTATIONS_QUERY, GET_EXTENDS_QUERY, GET_FIELD_RETURN_QUERY,
        GET_FUNCTION_RETURN_QUERY, GET_GROOVYDOC_QUERY, GET_IMPLEMENTS_QUERY, GET_IMPORTS_QUERY,
        GET_MODIFIERS_QUERY, GET_PACKAGE_NAME_QUERY, GET_PARAMETERS_QUERY, GET_SHORT_NAME_QUERY,
        GET_TYPE_QUERY, IDENT_QUERY,
    },
};

mod queries;

pub struct GroovySupport;

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}

impl GroovySupport {
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
                    ("method_name", Some("method_qualifier")),
                    ("this_method_name", Some("this_qualifier")),
                    ("field_name", Some("field_qualifier")),
                    ("arg_name", None),
                    ("var_decl", None),
                    ("constructor_type", None),
                    ("type_arg", None),
                    ("cast_type", None),
                    ("class_name", None),
                    ("interface_name", None),
                    ("function_name", None),
                    ("field_decl_name", None),
                    (
                        "scoped_constructor_type",
                        Some("scoped_constructor_qualifier"),
                    ),
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
        let name_idx = query.capture_index_for_name("import_name");
        let full_import_idx = query.capture_index_for_name("full_import");

        let name_cap = match_.captures.iter().find(|c| Some(c.index) == name_idx)?;
        let full_import_cap = match_
            .captures
            .iter()
            .find(|c| Some(c.index) == full_import_idx)?;

        let name_node = name_cap.node;
        let full_import_node = full_import_cap.node;

        if !node_contains_position(&name_node, position) {
            return None;
        }

        let full_text = &content[full_import_node.byte_range()];
        let name_text = &content[name_node.byte_range()];
        let scope_text = full_text.strip_suffix(&format!(".{}", name_text))?;

        Some((name_text.to_string(), Some(scope_text.to_string())))
    }

    fn traverse_scope_nodes<F>(
        &self,
        scope_node: Node,
        content: &str,
        reference_byte: usize,
        process_node: &mut F,
    ) where
        F: FnMut(Node, &str) -> bool, // true = continue, false = stop
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
                    "variable_declaration" | "field_declaration" | "parameter" => {
                        if !process_node(child, content) {
                            return;
                        }
                    }
                    "expression_statement"
                    | "assignment_expression"
                    | "object_creation_expression"
                    | "parameters" => {
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

    fn find_in_current_scope(
        &self,
        scope_node: Node,
        content: &str,
        var_name: &str,
        reference_byte: usize,
    ) -> Option<(Option<String>, Position)> {
        let mut result = None;
        let mut process_node = |child: Node, content: &str| -> bool {
            if self.declares_variable(child, content, var_name) {
                let var_type = if let Some(type_node) = child.child_by_field_name("type") {
                    let type_text = type_node
                        .utf8_text(content.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                    match type_text.as_deref() {
                        Some("def") => self.infer_type_from_declarator(&child, content),
                        _ => type_text,
                    }
                } else {
                    self.infer_type_from_declarator(&child, content)
                };

                if let Some((_, var_position)) =
                    ts_helper::get_one_with_position(&child, content, &DECLARES_VARIABLE_QUERY)
                {
                    result = Some((var_type, var_position));
                    return false;
                }
            }
            true
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
            let names = ts_helper::get_many(&child, content, &DECLARES_VARIABLE_QUERY, Some(1));
            let var_type = if let Some(type_node) = child.child_by_field_name("type") {
                let type_text = content[type_node.start_byte()..type_node.end_byte()].to_string();
                if type_text == "def" {
                    self.infer_type_from_declarator(&child, content)
                } else {
                    Some(type_text)
                }
            } else {
                self.infer_type_from_declarator(&child, content)
            };

            for name in names {
                results.push((name, var_type.clone()));
            }

            true
        };

        self.traverse_scope_nodes(scope_node, content, reference_byte, &mut process_node);
    }

    fn declares_variable(&self, node: Node, content: &str, var_name: &str) -> bool {
        let names = ts_helper::get_many(&node, content, &DECLARES_VARIABLE_QUERY, Some(1));
        names.iter().any(|name| name == var_name)
    }

    /// Infer a type from the initializer value of a `variable_declarator` child.
    /// Used for `def` variables that have no explicit type annotation.
    fn infer_type_from_declarator(&self, var_decl_node: &Node, content: &str) -> Option<String> {
        let declarator = var_decl_node.child_by_field_name("declarator")?;
        let value = declarator.child_by_field_name("value")?;
        Self::infer_type_from_value_node(&value, content)
    }

    /// Extracts a `#`-separated chain qualifier from a `method_invocation` or `field_access`
    /// expression. Returns `None` if the expression is not a supported chain pattern.
    /// Examples:
    ///   `Bar.create()`  → `Some("Bar#create")`
    ///   `foo.bar().baz()` → `Some("foo#bar#baz")`
    ///   `it.name`       → `Some("it#name")`
    fn extract_invocation_chain(node: &Node, content: &str) -> Option<String> {
        match node.kind() {
            "identifier" => node
                .utf8_text(content.as_bytes())
                .ok()
                .map(|s| s.to_string()),
            "method_invocation" => {
                let obj = node.child_by_field_name("object")?;
                let name_node = node.child_by_field_name("name")?;
                let obj_chain_raw = Self::extract_invocation_chain(&obj, content)?;
                // Strip lambda body info from receiver chain to avoid propagation.
                let obj_chain = if let Some(idx) = obj_chain_raw.find("__lb__") {
                    obj_chain_raw[..idx].to_string()
                } else {
                    obj_chain_raw
                };
                let method_name = name_node.utf8_text(content.as_bytes()).ok()?;
                let chain = format!("{}#{}", obj_chain, method_name);
                if let Some(body_info) = Self::extract_closure_body_chain(node, content) {
                    Some(format!("{}__lb__{}", chain, body_info))
                } else {
                    Some(chain)
                }
            }
            "field_access" => {
                let obj = node.child_by_field_name("object")?;
                let field_node = node.child_by_field_name("field")?;
                let obj_chain = Self::extract_invocation_chain(&obj, content)?;
                let field_name = field_node.utf8_text(content.as_bytes()).ok()?;
                Some(format!("{}#{}", obj_chain, field_name))
            }
            _ => None,
        }
    }

    /// If `method_invoc` has a closure argument, returns `"param|body_chain"`.
    fn extract_closure_body_chain(method_invoc: &Node, content: &str) -> Option<String> {
        // Closure may be the `closure:` field or inside the `arguments:` argument_list.
        let closure = method_invoc
            .child_by_field_name("closure")
            .or_else(|| {
                let args = method_invoc.child_by_field_name("arguments")?;
                let mut ac = args.walk();
                args.named_children(&mut ac)
                    .find(|n| n.kind() == "closure")
            })?;

        let mut cc = closure.walk();
        let closure_children: Vec<_> = closure.children(&mut cc).collect();

        // Param name: last identifier in the first closure_parameter, or "it" for
        // closures with no declared parameters (Groovy implicit parameter).
        let param_name = closure_children
            .iter()
            .find(|n| n.kind() == "closure_parameter")
            .and_then(|cp| {
                let mut pc = cp.walk();
                let children: Vec<_> = cp.children(&mut pc).collect();
                children
                    .iter()
                    .rev()
                    .find(|n| n.kind() == "identifier")
                    .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "it".to_string());

        // Body: last named non-closure_parameter child.
        let last_body = closure_children
            .iter()
            .filter(|n| n.is_named() && n.kind() != "closure_parameter")
            .last()?;

        // Unwrap expression_statement.
        let expr = if last_body.kind() == "expression_statement" {
            last_body.named_child(0)?
        } else {
            *last_body
        };

        let body_chain = Self::extract_invocation_chain(&expr, content)?;
        Some(format!("{}|{}", param_name, body_chain))
    }

    fn infer_type_from_value_node(value_node: &Node, content: &str) -> Option<String> {
        match value_node.kind() {
            "object_creation_expression" => {
                let type_node = value_node.child_by_field_name("type")?;
                type_node
                    .utf8_text(content.as_bytes())
                    .ok()
                    .map(|s| s.to_string())
            }
            "method_invocation" => Self::extract_invocation_chain(value_node, content),
            "string_literal" | "gstring" | "text_block" => Some("String".to_string()),
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal" => {
                let text = value_node.utf8_text(content.as_bytes()).ok()?;
                if text.ends_with('l') || text.ends_with('L') {
                    Some("Long".to_string())
                } else {
                    Some("Integer".to_string())
                }
            }
            "decimal_floating_point_literal" | "hex_floating_point_literal" => {
                let text = value_node.utf8_text(content.as_bytes()).ok()?;
                let lower = text.to_lowercase();
                if lower.ends_with('f') {
                    Some("Float".to_string())
                } else if lower.ends_with('d') {
                    Some("Double".to_string())
                } else {
                    Some("BigDecimal".to_string())
                }
            }
            "true" | "false" => Some("Boolean".to_string()),
            "array_literal" => Some("List".to_string()),
            "map_literal" => Some("Map".to_string()),
            _ => None,
        }
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

    /// When `var_name` is an untyped closure parameter, returns a `__cp__:…` marker.
    fn find_closure_param_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(Option<String>, Position)> {
        let mut node = get_node_at_position(tree, content, position)?;

        loop {
            if node.kind() == "closure" {
                let mut cursor = node.walk();
                let mut closure_param_index = 0usize;
                let mut has_explicit_params = false;
                for child in node.children(&mut cursor) {
                    if child.kind() != "closure_parameter" {
                        continue;
                    }
                    has_explicit_params = true;
                    // The last identifier child is the parameter name; a preceding
                    // type_identifier child (if present) is the explicit type.
                    let mut pc = child.walk();
                    let children: Vec<_> = child.children(&mut pc).collect();
                    let name_node = children.iter().rev().find(|n| n.kind() == "identifier")?;
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    if name == var_name {
                        let explicit_type = children
                            .iter()
                            .find(|n| n.kind() == "type_identifier")
                            .and_then(|t| t.utf8_text(content.as_bytes()).ok())
                            .map(|s| s.to_string());
                        let decl_pos = Position {
                            line: name_node.start_position().row as u32,
                            character: name_node.start_position().column as u32,
                        };
                        if let Some(t) = explicit_type {
                            return Some((Some(t), decl_pos));
                        }
                        let type_str =
                            self.build_closure_param_marker(&node, content, closure_param_index)?;
                        return Some((Some(type_str), decl_pos));
                    }
                    closure_param_index += 1;
                }

                // Groovy implicit `it`: a closure with no declared parameters uses `it` as
                // the implicit first parameter.
                if !has_explicit_params && var_name == "it" {
                    let type_str = self.build_closure_param_marker(&node, content, 0)?;
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

    /// Builds a `__cp__:receiver_chain:method_name:method_param_idx:closure_param_idx`
    /// marker for a closure parameter at `closure_param_index` inside `closure_node`.
    fn build_closure_param_marker(
        &self,
        closure_node: &Node,
        content: &str,
        closure_param_index: usize,
    ) -> Option<String> {
        let parent = closure_node.parent()?;
        let (method_invoc, method_param_idx) = if parent.kind() == "method_invocation" {
            // Trailing closure: items.each { ... }
            // Count regular arguments in the argument_list field (if any).
            let arg_count = parent
                .child_by_field_name("arguments")
                .map(|al| al.named_child_count())
                .unwrap_or(0);
            (parent, arg_count)
        } else if parent.kind() == "argument_list" {
            // Closure inside argument list: items.each({ ... })
            let grandparent = parent.parent()?;
            if grandparent.kind() != "method_invocation" {
                return None;
            }
            // Find the index of the closure in the argument list.
            let mut idx = 0usize;
            let mut pc = parent.walk();
            for arg in parent.named_children(&mut pc) {
                if arg.id() == closure_node.id() {
                    break;
                }
                idx += 1;
            }
            (grandparent, idx)
        } else {
            return None;
        };

        let receiver = method_invoc.child_by_field_name("object")?;
        let name_node = method_invoc.child_by_field_name("name")?;
        let method_name = name_node.utf8_text(content.as_bytes()).ok()?;
        let receiver_chain = Self::extract_invocation_chain(&receiver, content)?;

        Some(format!(
            "__cp__:{}:{}:{}:{}",
            receiver_chain, method_name, method_param_idx, closure_param_index
        ))
    }
}

impl LanguageSupport for GroovySupport {
    fn get_language(&self) -> Language {
        Language::Groovy
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_groovy::language()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        thread_local! {
            static PARSER: RefCell<Parser> = RefCell::new({
                let mut p = Parser::new();
                p.set_language(&tree_sitter_groovy::language()).unwrap();
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
            "class_declaration" => Some(NodeKind::Class),
            "interface_declaration" => Some(NodeKind::Interface),
            "enum_declaration" => Some(NodeKind::Enum),
            "function_declaration" => Some(NodeKind::Function),
            "field_declaration" => node.parent().and_then(|parent| match parent.kind() {
                "class_body" => Some(NodeKind::Field),
                _ => None,
            }),
            "annotation_type_declaration" => Some(NodeKind::Annotation),
            "constant_declaration" => Some(NodeKind::Field),
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_SHORT_NAME_QUERY)
            .map(|name| name.trim_matches(|c| c == '\'' || c == '"').to_string())
    }

    fn get_extends(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_EXTENDS_QUERY)
    }

    fn get_implements(&self, node: &Node, source: &str) -> Vec<String> {
        ts_helper::get_many(node, source, &GET_IMPLEMENTS_QUERY, Some(1))
    }

    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String> {
        let node_kind = self.get_kind(node);

        match node_kind {
            Some(_) => ts_helper::get_many(node, source, &GET_MODIFIERS_QUERY, Some(1)),
            None => Vec::new(),
        }
    }

    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String> {
        let node_kind = self.get_kind(node);

        match node_kind {
            Some(_) => ts_helper::get_many(node, source, &GET_ANNOTATIONS_QUERY, Some(1)),
            None => Vec::new(),
        }
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_GROOVYDOC_QUERY)
    }

    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>> {
        if let Some(NodeKind::Function) = self.get_kind(node) {
            let params = ts_helper::get_many(node, source, &GET_PARAMETERS_QUERY, Some(1))
                .into_iter()
                .map(|p| ts_helper::parse_parameter(&p))
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
            ts_helper::get_many(&tree.root_node(), source, &GET_IMPORTS_QUERY, Some(1))
                .into_iter()
                .map(|i| i.strip_prefix("import ").unwrap_or_default().to_string())
                .collect::<Vec<String>>();

        GROOVY_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(explicit_imports)
            .collect()
    }

    fn get_implicit_imports(&self) -> Vec<String> {
        GROOVY_IMPLICIT_IMPORTS
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
            let kind = current.kind();

            if kind == "method_invocation" {
                let mut cursor = current.walk();
                for child in current.children(&mut cursor) {
                    if child.kind() == "argument_list" {
                        return Some(self.parse_argument_list(&child, content));
                    }
                }

                // TODO: handle closures and object method invocation
                return Some(vec![]);
            }

            current = current.parent()?;
        }
    }

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String> {
        let point = Point::new(position.line as usize, position.character as usize);
        let mut node = tree.root_node().descendant_for_point_range(point, point)?;

        loop {
            match node.kind() {
                "map_literal" => return Some("Map".to_string()),
                "array_literal" => return Some("List".to_string()),

                // Numeric literals
                "decimal_integer_literal"
                | "hex_integer_literal"
                | "octal_integer_literal"
                | "binary_integer_literal" => {
                    // Check for long suffix (l or L)
                    if let Ok(text) = node.utf8_text(content.as_bytes())
                        && (text.ends_with('l') || text.ends_with('L'))
                    {
                        return Some("Long".to_string());
                    }
                    return Some("Integer".to_string());
                }

                "decimal_floating_point_literal" | "hex_floating_point_literal" => {
                    if let Ok(text) = node.utf8_text(content.as_bytes()) {
                        let lower = text.to_lowercase();
                        if lower.ends_with('f') {
                            return Some("Float".to_string());
                        } else if lower.ends_with('d') {
                            return Some("Double".to_string());
                        }
                    }
                    return Some("BigDecimal".to_string());
                }

                "true" | "false" => return Some("Boolean".to_string()),

                "string_literal" | "text_block" => return Some("String".to_string()),

                "null_literal" => return None,

                "regex_literal" => return Some("Pattern".to_string()),

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
            name: (identifier) @receiver
            body: (class_body (function_declaration) @method))
          (interface_declaration 
            name: (identifier) @receiver
            body: (interface_body (function_declaration) @method))
          (enum_declaration 
            name: (identifier) @receiver
            body: (enum_body (function_declaration) @method))
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
                if parent.kind() == "class_declaration" || parent.kind() == "enum_declaration" {
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

        // var_name was not found as a regular local variable — check if it is a
        // closure parameter inside an enclosing closure.
        self.find_closure_param_declaration(tree, content, var_name, position)
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
