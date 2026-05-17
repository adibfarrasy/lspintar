use lsp_core::{
    language_support::{
        IdentResult, LanguageSupport, ParameterResult, ParseResult,
    },
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, collect_syntax_errors},
};
use std::{cell::RefCell, collections::HashSet, fs, path::Path, sync::LazyLock};

use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Node, Parser, Tree};

use crate::{
    constants::SCALA_IMPLICIT_IMPORTS,
    support::queries::{
        GET_IMPORTS_QUERY, GET_PACKAGE_NAME_QUERY, GET_SHORT_NAME_QUERY,
        GET_VAL_SHORT_NAME_QUERY,
    },
};

mod queries;
#[cfg(test)]
mod tests;

pub struct ScalaSupport;

impl Default for ScalaSupport {
    fn default() -> Self {
        Self::new()
    }
}

impl ScalaSupport {
    pub fn new() -> Self {
        Self
    }
}

/// Extract every parent type written under an `extends_clause`, in source order.
/// The first entry is the `extends T` target; subsequent entries are the
/// `with M1 with M2` mixins.  Filters out the literal `extends` / `with`
/// keyword tokens and any constructor `arguments` field.
fn extends_clause_types(extend: Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut c = extend.walk();
    for child in extend.children(&mut c) {
        match child.kind() {
            "type_identifier"
            | "generic_type"
            | "stable_type_identifier"
            | "projected_type"
            | "compound_type"
            | "infix_type"
            | "applied_constructor_type"
            | "function_type"
            | "annotated_type"
            | "literal_type" => {
                if let Ok(t) = child.utf8_text(source.as_bytes()) {
                    out.push(t.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

// Scala 2.13 + 3 reserved words (union; both dialects share most of these).
static SCALA_RESERVED: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "abstract", "case", "catch", "class", "def", "do", "else", "enum", "export", "extends",
        "false", "final", "finally", "for", "forSome", "given", "if", "implicit", "import",
        "lazy", "macro", "match", "new", "null", "object", "override", "package", "private",
        "protected", "return", "sealed", "super", "then", "this", "throw", "trait", "true",
        "try", "type", "using", "val", "var", "while", "with", "yield",
    ]
    .into_iter()
    .collect()
});

impl LanguageSupport for ScalaSupport {
    fn get_language(&self) -> Language {
        Language::Scala
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_scala::LANGUAGE.into()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        thread_local! {
            static PARSER: RefCell<Parser> = RefCell::new({
                let mut p = Parser::new();
                p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
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
        let r = node.range();
        Some(Range {
            start: Position {
                line: r.start_point.row as u32,
                character: r.start_point.column as u32,
            },
            end: Position {
                line: r.end_point.row as u32,
                character: r.end_point.column as u32,
            },
        })
    }

    fn get_ident_range(&self, node: &Node) -> Option<Range> {
        let ident_node = match node.kind() {
            "class_definition"
            | "object_definition"
            | "trait_definition"
            | "enum_definition"
            | "function_definition"
            | "function_declaration"
            | "type_definition"
            | "given_definition" => node.child_by_field_name("name")?,
            "val_definition" | "var_definition" => node.child_by_field_name("pattern")?,
            "val_declaration" | "var_declaration" => node.child_by_field_name("name")?,
            _ => node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier" || n.kind() == "type_identifier")?,
        };

        let r = ident_node.range();
        Some(Range {
            start: Position {
                line: r.start_point.row as u32,
                character: r.start_point.column as u32,
            },
            end: Position {
                line: r.end_point.row as u32,
                character: r.end_point.column as u32,
            },
        })
    }

    fn get_package_name(&self, tree: &Tree, content: &str) -> Option<String> {
        ts_helper::get_one(&tree.root_node(), content, &GET_PACKAGE_NAME_QUERY)
    }

    fn get_kind(&self, node: &Node) -> Option<NodeKind> {
        match node.kind() {
            "class_definition" | "object_definition" => Some(NodeKind::Class),
            "trait_definition" => Some(NodeKind::Interface),
            "enum_definition" => Some(NodeKind::Enum),
            "function_definition" | "function_declaration" => Some(NodeKind::Function),
            "val_definition" | "var_definition" | "val_declaration" | "var_declaration"
            | "given_definition" => Some(NodeKind::Field),
            "type_definition" => Some(NodeKind::Class),
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        match self.get_kind(node) {
            Some(NodeKind::Field) => {
                ts_helper::get_one(node, source, &GET_VAL_SHORT_NAME_QUERY)
            }
            Some(_) => ts_helper::get_one(node, source, &GET_SHORT_NAME_QUERY),
            None => None,
        }
    }

    // --- Hierarchy + metadata (Phase 2).

    fn get_extends(&self, node: &Node, source: &str) -> Option<String> {
        let extend = node.child_by_field_name("extend")?;
        extends_clause_types(extend, source)
            .into_iter()
            .next()
    }

    fn get_implements(&self, node: &Node, source: &str) -> Vec<String> {
        let Some(extend) = node.child_by_field_name("extend") else {
            return Vec::new();
        };
        let mut types = extends_clause_types(extend, source);
        if !types.is_empty() {
            types.remove(0);
        }
        types
    }

    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                "modifiers" => {
                    let mut mc = child.walk();
                    for m in child.children(&mut mc) {
                        if let Ok(t) = m.utf8_text(source.as_bytes()) {
                            let trimmed = t.trim();
                            if !trimmed.is_empty() {
                                out.push(trimmed.to_string());
                            }
                        }
                    }
                }
                // `case`, `override`, `opaque`, `final`, `sealed`, `lazy`, `implicit`,
                // `abstract`, `inline`, `transparent`, `open`, `infix` may also appear
                // as direct keyword-token children outside the `modifiers` node.
                "case" | "override" | "opaque" | "final" | "sealed" | "lazy"
                | "implicit" | "abstract" | "inline" | "transparent" | "open" | "infix" => {
                    out.push(child.kind().to_string());
                }
                _ => {}
            }
        }
        out
    }

    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() == "annotation" {
                if let Ok(t) = child.utf8_text(source.as_bytes()) {
                    out.push(t.to_string());
                }
            }
        }
        out
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        // Scaladoc lives in a `block_comment` previous sibling that opens with `/**`.
        let mut cursor = node.prev_sibling();
        while let Some(sib) = cursor {
            match sib.kind() {
                "block_comment" => {
                    let text = sib.utf8_text(source.as_bytes()).ok()?;
                    if text.starts_with("/**") {
                        return Some(text.to_string());
                    }
                    return None;
                }
                // Whitespace / line-comment siblings: keep walking back.
                "line_comment" | "comment" => cursor = sib.prev_sibling(),
                _ => return None,
            }
        }
        None
    }

    fn get_parameters(
        &self,
        node: &Node,
        source: &str,
    ) -> Option<Vec<ParameterResult>> {
        let kind = self.get_kind(node)?;
        if !matches!(kind, NodeKind::Function | NodeKind::Class | NodeKind::Enum | NodeKind::Interface)
        {
            return None;
        }
        // Functions use `parameters`; classes/traits/enums use `class_parameters`.
        let mut c = node.walk();
        let mut params: Vec<ParameterResult> = Vec::new();
        for child in node.children(&mut c) {
            if child.kind() == "parameters" || child.kind() == "class_parameters" {
                let mut pc = child.walk();
                for p in child.children(&mut pc) {
                    if p.kind() == "parameter" || p.kind() == "class_parameter" {
                        let name = p
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let ty = p
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string());
                        let default = p
                            .child_by_field_name("default_value")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.trim_matches('"').to_string());
                        params.push((name, ty, default));
                    }
                }
            }
        }
        if params.is_empty() {
            // function_definition with empty `()` still has a parameters node — return Some(vec![]).
            // function_definition without any parens (e.g. `def x: Int = 1`) returns Some(vec![]).
            return Some(Vec::new());
        }
        Some(params)
    }

    fn get_return(&self, node: &Node, source: &str) -> Option<String> {
        match self.get_kind(node)? {
            NodeKind::Function => node
                .child_by_field_name("return_type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())),
            NodeKind::Field => node
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())),
            _ => None,
        }
    }

    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String> {
        let imports = ts_helper::get_many(&tree.root_node(), source, &GET_IMPORTS_QUERY, None);
        let mut out: Vec<String> = imports
            .into_iter()
            .map(|raw| {
                // Strip the leading `import ` keyword and any trailing semicolon/newline noise.
                raw.trim_start_matches("import")
                    .trim()
                    .trim_end_matches(';')
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();
        out.extend(self.get_implicit_imports());
        out
    }

    fn get_implicit_imports(&self) -> Vec<String> {
        SCALA_IMPLICIT_IMPORTS.iter().map(|s| s.to_string()).collect()
    }

    // --- Position / type resolution (Phase 3): all stubs.

    fn get_type_at_position(
        &self,
        _node: Node,
        _content: &str,
        _position: &Position,
    ) -> Option<String> {
        None
    }

    fn find_ident_at_position(
        &self,
        _tree: &Tree,
        _content: &str,
        _position: &Position,
    ) -> Option<IdentResult> {
        None
    }

    fn find_variable_type(
        &self,
        _tree: &Tree,
        _content: &str,
        _var_name: &str,
        _position: &Position,
    ) -> Option<String> {
        None
    }

    fn find_variable_declaration(
        &self,
        _tree: &Tree,
        _content: &str,
        _var_name: &str,
        _position: &Position,
    ) -> Option<(Option<String>, Position)> {
        None
    }

    fn find_declarations_in_scope(
        &self,
        _tree: &Tree,
        _content: &str,
        _position: &Position,
    ) -> Vec<(String, Option<String>)> {
        Vec::new()
    }

    fn extract_call_arguments(
        &self,
        _tree: &Tree,
        _content: &str,
        _position: &Position,
    ) -> Option<Vec<(String, Position)>> {
        None
    }

    fn get_literal_type(
        &self,
        _tree: &Tree,
        _content: &str,
        _position: &Position,
    ) -> Option<String> {
        None
    }

    fn get_method_receiver_and_params(
        &self,
        _node: Node,
        _content: &str,
        _position: &Position,
    ) -> Option<(String, Vec<String>)> {
        None
    }

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        collect_syntax_errors(tree.root_node(), source, &mut diagnostics);
        diagnostics
    }

    fn reserved_keywords(&self) -> &'static HashSet<&'static str> {
        &SCALA_RESERVED
    }
}
