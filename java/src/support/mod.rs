use lsp_core::{
    language_support::{CallArgData, ClassDeclarationData, GenericTypeUsage, IdentResult, LanguageSupport, MemberAccessData, MethodCallSiteData, NarrowingCandidateData, ObjectCreationData, OverrideMethodData, ParameterResult, ParseResult},
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, collect_syntax_errors, get_node_at_position, node_contains_position},
};
use std::{cell::RefCell, fs, path::Path};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::{
    constants::JAVA_IMPLICIT_IMPORTS,
    support::queries::{
        CLASS_METHOD_NAMES_QUERY, DECLARED_TYPES_QUERY, DECLARES_VARIABLE_QUERY,
        FUNCTION_WITH_RETURN_QUERY, GET_ANNOTATIONS_QUERY, GET_EXTENDS_QUERY,
        GET_FIELD_RETURN_QUERY, GET_FIELD_SHORT_NAME_QUERY, GET_FUNCTION_RETURN_QUERY,
        GET_GENERIC_TYPE_USAGES_QUERY, GET_IMPLEMENTS_QUERY, GET_IMPORTS_QUERY,
        GET_JAVADOC_QUERY, GET_MEMBER_ACCESSES_QUERY, GET_MODIFIERS_QUERY,
        GET_METHOD_CALL_SITES_QUERY, GET_NARROWING_CANDIDATES_QUERY, GET_OBJECT_CREATIONS_QUERY, GET_OVERRIDE_METHODS_QUERY,
        GET_PACKAGE_NAME_QUERY, GET_PARAMETERS_QUERY, GET_SHORT_NAME_QUERY, GET_TYPE_QUERY,
        GET_TYPE_REFS_QUERY, IDENT_QUERY,
    },
};

mod queries;

pub struct JavaSupport;

impl Default for JavaSupport {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaSupport {
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
                        Some("var") => self.infer_type_from_declarator(&child, content),
                        _ => type_text,
                    }
                } else {
                    None
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
                if type_text == "var" {
                    self.infer_type_from_declarator(&child, content)
                } else {
                    Some(type_text)
                }
            } else {
                None
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
    /// Used for `var` declarations where the type must be derived from the expression.
    fn infer_type_from_declarator(&self, var_decl_node: &Node, content: &str) -> Option<String> {
        let declarator = var_decl_node.child_by_field_name("declarator")?;
        let value = declarator.child_by_field_name("value")?;
        Self::infer_type_from_value_node(&value, content)
    }

    /// Extracts a `#`-separated chain qualifier from a `method_invocation` expression.
    /// Returns `None` if the expression is not a supported chain pattern.
    /// Examples:
    ///   `Bar.create()`  → `Some("Bar#create")`
    ///   `foo.bar().baz()` → `Some("foo#bar#baz")`
    fn extract_invocation_chain(node: &Node, content: &str) -> Option<String> {
        match node.kind() {
            "identifier" => node
                .utf8_text(content.as_bytes())
                .ok()
                .map(|s| s.to_string()),
            "method_invocation" => {
                let obj = node.child_by_field_name("object")?;
                let name_node = node.child_by_field_name("name")?;
                let obj_chain = Self::extract_invocation_chain(&obj, content)?;
                let method_name = name_node.utf8_text(content.as_bytes()).ok()?;
                Some(format!("{}#{}", obj_chain, method_name))
            }
            _ => None,
        }
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
            "string_literal" | "text_block" => Some("String".to_string()),
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
                if text.to_lowercase().ends_with('f') {
                    Some("Float".to_string())
                } else {
                    Some("Double".to_string())
                }
            }
            "true" | "false" => Some("Boolean".to_string()),
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
}

/// Returns true if `node` contains a `return_statement` without crossing into
/// inner class or lambda boundaries.
fn has_return_in_block(node: tree_sitter::Node) -> bool {
    if node.kind() == "return_statement" {
        return true;
    }
    if matches!(
        node.kind(),
        "class_declaration"
            | "enum_declaration"
            | "lambda_expression"
            | "anonymous_class_body"
    ) {
        return false;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if has_return_in_block(child) {
            return true;
        }
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
        if node.kind() == "cast_expression" {
            if let Some(type_node) = node.child_by_field_name("type") {
                if type_node.kind() == "generic_type" {
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
        if kind == "class_body" || kind == "enum_body" {
            check_body_for_dup_sigs(node, bytes, diagnostics);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
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

            // skip void methods
            if ret_cap.node.kind() == "void_type" {
                return;
            }

            // parent of the name node is the function_declaration
            let Some(func_node) = name_cap.node.parent() else {
                return;
            };

            // find the block body; absent means abstract/interface method
            let mut block_opt = None;
            let mut c = func_node.walk();
            for child in func_node.children(&mut c) {
                if child.kind() == "block" {
                    block_opt = Some(child);
                    break;
                }
            }
            let Some(block) = block_opt else {
                return;
            };

            if !has_return_in_block(block) {
                let range = node_to_range(&name_cap.node);
                let name = name_cap.node.utf8_text(bytes).unwrap_or("?");
                diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                    range,
                    severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "missing_return_statement".to_string(),
                    )),
                    source: Some("lspintar".to_string()),
                    message: format!("Missing return statement in method '{name}'"),
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
            // strip "import " prefix and trailing ";"
            let fqn = text
                .trim_start_matches("import ")
                .trim_start_matches("static ")
                .trim_end_matches(';')
                .trim()
                .to_string();
            let range = node_to_range(&node);
            if let Some(_first_range) = seen.get(&fqn) {
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

    // Collect all import nodes with their ranges and simple names.
    let mut imports: Vec<(String, String, Range)> = Vec::new(); // (fqn, simple_name, range)
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
            let fqn = text
                .trim_start_matches("import ")
                .trim_start_matches("static ")
                .trim_end_matches(';')
                .trim()
                .to_string();
            // Wildcard imports always "used" — skip them.
            if fqn.ends_with(".*") {
                return;
            }
            let simple = fqn
                .split('.')
                .next_back()
                .unwrap_or(&fqn)
                .to_string();
            imports.push((fqn, simple, node_to_range(&node)));
        });

    if imports.is_empty() {
        return;
    }

    // Determine the byte offset where the import section ends so we only
    // search in the actual code body.
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
        // A word-boundary check: the simple name must appear as a standalone
        // identifier in the file body.
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
    let is_id = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
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

impl LanguageSupport for JavaSupport {
    fn get_language(&self) -> Language {
        Language::Java
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_java::LANGUAGE.into()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        thread_local! {
            static PARSER: RefCell<Parser> = RefCell::new({
                let mut p = Parser::new();
                p.set_language(&tree_sitter_java::LANGUAGE.into()).unwrap();
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
            Some(_) => ts_helper::get_many(node, source, &GET_ANNOTATIONS_QUERY, Some(1)),
            None => Vec::new(),
        }
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        ts_helper::get_one(node, source, &GET_JAVADOC_QUERY)
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
                .map(|i| {
                    i.strip_prefix("import ")
                        .unwrap_or_default()
                        .trim_end_matches(';')
                        .trim()
                        .to_string()
                })
                .collect::<Vec<String>>();

        JAVA_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(explicit_imports)
            .collect()
    }

    fn get_implicit_imports(&self) -> Vec<String> {
        JAVA_IMPLICIT_IMPORTS
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
                        }
                    }
                    return Some("Double".to_string());
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
            body: (enum_body (enum_body_declarations (function_declaration) @method)))
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
        None
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
                    let range = Range {
                        start: tower_lsp::lsp_types::Position {
                            line: node.start_position().row as u32,
                            character: node.start_position().column as u32,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line: node.end_position().row as u32,
                            character: node.end_position().column as u32,
                        },
                    };
                    refs.push((text.to_string(), range));
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

        // DECLARED_TYPES_QUERY captures the name identifier node of each type declaration.
        // We get the parent node (class_declaration / enum_declaration) from each capture.
        cursor
            .matches(&DECLARED_TYPES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(name_cap) = m.captures.first() else { return; };
                let name_node = name_cap.node;
                let Some(type_node) = name_node.parent() else { return; };

                // Only class and enum declarations can have unimplemented methods.
                let kind = type_node.kind();
                if kind != "class_declaration" && kind != "enum_declaration" {
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

                // Collect method names defined directly in this class body.
                // We scope the query to just the class body node to avoid picking up
                // methods from inner classes.
                let mut defined_method_names = std::collections::HashSet::new();
                for i in 0..type_node.child_count() {
                    let Some(child) = type_node.child(i) else { continue };
                    if child.kind() == "class_body" || child.kind() == "enum_body" {
                        let mut method_cursor = QueryCursor::new();
                        // Limit depth so we don't descend into inner class bodies.
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

    fn get_object_creations(&self, tree: &Tree, source: &str) -> Vec<ObjectCreationData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();

        cursor
            .matches(&GET_OBJECT_CREATIONS_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                // The query has a single @type_name capture regardless of which branch matched.
                if let Some(cap) = m.captures.first() {
                    if let Ok(type_name) = cap.node.utf8_text(bytes) {
                        results.push(ObjectCreationData {
                            type_name: type_name.to_string(),
                            range: node_to_range(&cap.node),
                        });
                    }
                }
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
                // Count named children of type_arguments that are actual type nodes (not punctuation).
                let arg_count = args_cap
                    .node
                    .named_children(&mut args_cap.node.walk())
                    .filter(|n| n.kind() != "," && n.is_named())
                    .count();
                // The full generic_type node is the parent of @base.
                let Some(generic_node) = base_cap.node.parent() else { return; };
                results.push(GenericTypeUsage {
                    type_name: type_name.to_string(),
                    arg_count,
                    range: node_to_range(&generic_node),
                });
            });

        results
    }

    fn get_override_methods(&self, tree: &Tree, source: &str) -> Vec<OverrideMethodData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let ann_idx = GET_OVERRIDE_METHODS_QUERY.capture_index_for_name("ann");
        let ret_idx = GET_OVERRIDE_METHODS_QUERY.capture_index_for_name("ret");
        let name_idx = GET_OVERRIDE_METHODS_QUERY.capture_index_for_name("name");

        cursor
            .matches(&GET_OVERRIDE_METHODS_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(ann_cap) = m.captures.iter().find(|c| Some(c.index) == ann_idx) else {
                    return;
                };
                let Ok(ann_text) = ann_cap.node.utf8_text(bytes) else { return };
                if ann_text != "Override" {
                    return;
                }
                let Some(name_cap) = m.captures.iter().find(|c| Some(c.index) == name_idx) else {
                    return;
                };
                let Ok(method_name) = name_cap.node.utf8_text(bytes) else { return };
                let return_type = m
                    .captures
                    .iter()
                    .find(|c| Some(c.index) == ret_idx)
                    .and_then(|c| c.node.utf8_text(bytes).ok())
                    .filter(|s| *s != "void")
                    .map(|s| s.to_string());
                let Some(containing_class) =
                    find_containing_class(name_cap.node, bytes)
                else {
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

    fn get_narrowing_candidates(&self, tree: &Tree, source: &str) -> Vec<NarrowingCandidateData> {
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut results = Vec::new();
        let decl_type_idx = GET_NARROWING_CANDIDATES_QUERY.capture_index_for_name("decl_type");
        let rhs_idx = GET_NARROWING_CANDIDATES_QUERY.capture_index_for_name("rhs_name");

        cursor
            .matches(&GET_NARROWING_CANDIDATES_QUERY, tree.root_node(), bytes)
            .for_each(|m| {
                let Some(dt_cap) = m.captures.iter().find(|c| Some(c.index) == decl_type_idx)
                else {
                    return;
                };
                let Ok(decl_type) = dt_cap.node.utf8_text(bytes) else { return };
                if !is_numeric_primitive(decl_type) {
                    return;
                }
                let Some(rhs_cap) = m.captures.iter().find(|c| Some(c.index) == rhs_idx) else {
                    return;
                };
                let Ok(rhs_name) = rhs_cap.node.utf8_text(bytes) else { return };
                results.push(NarrowingCandidateData {
                    declared_type: decl_type.to_string(),
                    rhs_name: rhs_name.to_string(),
                    range: node_to_range(&rhs_cap.node),
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

                let mut args = Vec::new();
                let mut arg_cursor = args_cap.node.walk();
                for child in args_cap.node.children(&mut arg_cursor) {
                    if !child.is_named() {
                        continue; // skip "(" "," ")"
                    }
                    let node_kind = child.kind().to_string();
                    let text = child.utf8_text(bytes).unwrap_or("").to_string();
                    args.push(CallArgData {
                        node_kind,
                        text,
                        range: node_to_range(&child),
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
}

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

fn is_numeric_primitive(t: &str) -> bool {
    matches!(t, "byte" | "short" | "int" | "long" | "float" | "double")
}

/// Walks the AST looking for `block` nodes and, within each, linearly tracks variables
/// declared without an initializer, flagging any read that occurs before the first
/// plain assignment to that variable.
fn collect_variable_used_before_assignment(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if node.kind() == "block" {
        let mut uninit: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut cursor = node.walk();
        for stmt in node.children(&mut cursor) {
            java_process_block_stmt(stmt, bytes, &mut uninit, diagnostics);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_variable_used_before_assignment(child, bytes, diagnostics);
    }
}

fn java_process_block_stmt(
    stmt: Node,
    bytes: &[u8],
    uninit: &mut std::collections::HashSet<String>,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    match stmt.kind() {
        "variable_declaration" => {
            if let Some(declarator) = stmt.child_by_field_name("declarator") {
                let has_value = declarator.child_by_field_name("value").is_some();
                if let Some(name_node) = declarator.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(bytes) {
                        if has_value {
                            if let Some(val) = declarator.child_by_field_name("value") {
                                java_scan_reads(val, bytes, uninit, diagnostics);
                            }
                            uninit.remove(name);
                        } else {
                            uninit.insert(name.to_string());
                        }
                    }
                }
            }
        }
        "expression_statement" => {
            if let Some(inner) = stmt.child(0) {
                if inner.kind() == "assignment_expression" {
                    // child(1) is the operator token: "=" for pure, "+=" etc. for compound
                    let is_pure = inner
                        .child(1)
                        .and_then(|op| op.utf8_text(bytes).ok())
                        .map(|op| op == "=")
                        .unwrap_or(false);
                    if is_pure {
                        if let Some(lhs) = inner.child_by_field_name("left") {
                            if lhs.kind() == "identifier" {
                                if let Ok(name) = lhs.utf8_text(bytes) {
                                    if uninit.contains(name) {
                                        if let Some(rhs) = inner.child_by_field_name("right") {
                                            java_scan_reads(rhs, bytes, uninit, diagnostics);
                                        }
                                        uninit.remove(name);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                java_scan_reads(stmt, bytes, uninit, diagnostics);
            }
        }
        // For control-flow statements, recurse to discover nested blocks but do NOT
        // propagate uninit into them — we can't guarantee any branch executes.
        _ => {
            collect_variable_used_before_assignment(stmt, bytes, diagnostics);
        }
    }
}

fn java_scan_reads(
    node: Node,
    bytes: &[u8],
    uninit: &std::collections::HashSet<String>,
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    if uninit.is_empty() {
        return;
    }
    match node.kind() {
        "assignment_expression" => {
            let is_pure = node
                .child(1)
                .and_then(|op| op.utf8_text(bytes).ok())
                .map(|op| op == "=")
                .unwrap_or(false);
            if !is_pure {
                // Compound assignment reads the LHS too
                if let Some(lhs) = node.child_by_field_name("left") {
                    java_scan_reads(lhs, bytes, uninit, diagnostics);
                }
            }
            if let Some(rhs) = node.child_by_field_name("right") {
                java_scan_reads(rhs, bytes, uninit, diagnostics);
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
                java_scan_reads(child, bytes, uninit, diagnostics);
            }
        }
    }
}

/// Returns true when `value_kind` is a literal that clearly cannot be assigned to `declared_type`.
fn java_is_literal_type_mismatch(type_kind: &str, type_text: &str, value_kind: &str) -> bool {
    let is_int_lit = matches!(
        value_kind,
        "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
    );
    let is_float_lit = matches!(
        value_kind,
        "decimal_floating_point_literal" | "hex_floating_point_literal"
    );
    let is_bool_lit = matches!(value_kind, "true" | "false");
    let is_string_lit = matches!(value_kind, "string_literal" | "text_block");
    let is_char_lit = value_kind == "character_literal";

    match type_kind {
        // int / long / short / byte / char
        "integral_type" => is_bool_lit || is_string_lit || is_float_lit,
        // float / double
        "floating_point_type" => is_bool_lit || is_string_lit,
        "boolean_type" => is_int_lit || is_float_lit || is_string_lit || is_char_lit,
        "type_identifier" => {
            if type_text == "String" {
                is_int_lit || is_float_lit || is_bool_lit || is_char_lit
            } else {
                false
            }
        }
        _ => false,
    }
}

fn java_literal_type_name(value_kind: &str, value_text: &str) -> &'static str {
    match value_kind {
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal" => {
            if value_text.ends_with('l') || value_text.ends_with('L') {
                "long"
            } else {
                "int"
            }
        }
        "decimal_floating_point_literal" | "hex_floating_point_literal" => {
            if value_text.to_lowercase().ends_with('f') { "float" } else { "double" }
        }
        "true" | "false" => "boolean",
        "string_literal" | "text_block" => "String",
        "character_literal" => "char",
        _ => "unknown",
    }
}

fn collect_literal_type_mismatches(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    match node.kind() {
        "variable_declaration" => {
            check_java_var_decl_literal(node, bytes, diagnostics);
        }
        "return_statement" => {
            check_java_return_literal(node, bytes, diagnostics);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_literal_type_mismatches(child, bytes, diagnostics);
    }
}

fn check_java_var_decl_literal(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    let Some(type_node) = node.child_by_field_name("type") else { return };
    let type_kind = type_node.kind();
    let Ok(type_text) = type_node.utf8_text(bytes) else { return };
    // `var` requires inference — skip
    if type_text == "var" {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        let Some(value) = child.child_by_field_name("value") else { continue };
        let value_kind = value.kind();
        let value_text = value.utf8_text(bytes).unwrap_or("");
        if java_is_literal_type_mismatch(type_kind, type_text, value_kind) {
            let inferred = java_literal_type_name(value_kind, value_text);
            diagnostics.push(tower_lsp::lsp_types::Diagnostic {
                range: node_to_range(&value),
                severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "type_mismatch_assignment".to_string(),
                )),
                source: Some("lspintar".to_string()),
                message: format!(
                    "Type mismatch: cannot convert from '{inferred}' to '{type_text}'"
                ),
                ..Default::default()
            });
        }
    }
}

fn check_java_return_literal(
    node: Node,
    bytes: &[u8],
    diagnostics: &mut Vec<tower_lsp::lsp_types::Diagnostic>,
) {
    // First named child after "return" keyword is the expression
    let mut cursor = node.walk();
    let value = node
        .children(&mut cursor)
        .find(|n| n.is_named());
    let Some(value) = value else { return };
    let value_kind = value.kind();
    let value_text = value.utf8_text(bytes).unwrap_or("");

    // Walk up to find the enclosing function_declaration (stop at lambdas)
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "lambda_expression" | "anonymous_class_body" => return,
            "function_declaration" => {
                let Some(ret_node) = p.child_by_field_name("type") else { return };
                if ret_node.kind() == "void_type" {
                    return;
                }
                let ret_kind = ret_node.kind();
                let Ok(ret_text) = ret_node.utf8_text(bytes) else { return };
                if java_is_literal_type_mismatch(ret_kind, ret_text, value_kind) {
                    let inferred = java_literal_type_name(value_kind, value_text);
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
