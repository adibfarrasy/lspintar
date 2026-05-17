use lsp_core::{
    language_support::{
        IdentResult, LanguageSupport, ParameterResult, ParseResult,
    },
    languages::Language,
    node_kind::NodeKind,
    ts_helper::{self, collect_syntax_errors, get_node_at_position, node_contains_position},
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

/// Walk `node` and its descendants and return the innermost named node whose
/// span contains `position`.  If the start node is itself unnamed (e.g. a
/// keyword token like `true`), walks up to its nearest named ancestor first.
fn innermost_named_node<'a>(node: Node<'a>, position: &Position) -> Option<Node<'a>> {
    if !node_contains_position(&node, position) {
        return None;
    }
    let mut anchor = node;
    while !anchor.is_named() {
        anchor = anchor.parent()?;
    }
    Some(descend_to_named_leaf(anchor, position))
}

/// Recurse strictly downward from a named `anchor` to the innermost named
/// descendant containing `position`.  Never climbs back up the tree, so it
/// cannot loop on unnamed children.
fn descend_to_named_leaf<'a>(anchor: Node<'a>, position: &Position) -> Node<'a> {
    let mut best = anchor;
    let mut cursor = anchor.walk();
    for child in anchor.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        if node_contains_position(&child, position) {
            best = descend_to_named_leaf(child, position);
        }
    }
    best
}

/// Classify a literal node into its canonical Scala primitive type name.
/// Classification is by node kind alone — suffix-aware refinement (`L`, `f`)
/// would require access to the source slice and is left to the caller.
fn literal_type_for(node: &Node) -> Option<String> {
    match node.kind() {
        "integer_literal" => Some("Int".to_string()),
        "floating_point_literal" => Some("Double".to_string()),
        "boolean_literal" => Some("Boolean".to_string()),
        "string" | "interpolated_string_expression" => Some("String".to_string()),
        "character_literal" => Some("Char".to_string()),
        "null_literal" => None,
        _ => None,
    }
}

fn nearest_ancestor<'a>(mut node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    loop {
        if node.kind() == kind {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn first_child_with_kind<'a>(parent: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut c = parent.walk();
    parent.children(&mut c).find(|n| n.kind() == kind)
}

/// Locate the in-scope declaration of `var_name` reachable from `position`.
/// Walks outward from the position node, scanning each enclosing block /
/// template body for matching `val`/`var`/`parameter`/`enumerator` /
/// `case_clause` bindings.  Returns the declared type (when present) and
/// the start position of the binding identifier.
fn find_variable_declaration_impl(
    tree: &Tree,
    content: &str,
    var_name: &str,
    position: &Position,
) -> Option<(Option<String>, Position)> {
    let node = get_node_at_position(tree, content, position)?;
    let mut cursor: Option<Node> = Some(node);
    while let Some(n) = cursor {
        if let Some(found) = scan_node_for_declaration(n, content, var_name, Some(position)) {
            return Some(found);
        }
        cursor = n.parent();
    }
    // Fall back to scanning the whole tree (e.g. top-level vals in same file).
    scan_node_for_declaration(tree.root_node(), content, var_name, None)
}

/// Scan `node`'s named children (and matched descendants) for a binding of
/// `var_name`.  When `before` is `Some`, only bindings whose identifier
/// starts at or before that position are returned (forward declarations
/// inside the same scope are visible too).
fn scan_node_for_declaration(
    node: Node,
    content: &str,
    var_name: &str,
    before: Option<&Position>,
) -> Option<(Option<String>, Position)> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "val_definition" | "var_definition" => {
                if let Some(name_node) = child.child_by_field_name("pattern") {
                    if name_node.kind() == "identifier"
                        && name_node.utf8_text(content.as_bytes()).ok() == Some(var_name)
                    {
                        if pos_ok(name_node, before) {
                            let ty = child
                                .child_by_field_name("type")
                                .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                                .map(|s| s.to_string());
                            return Some((ty, node_start_position(name_node)));
                        }
                    }
                }
            }
            "val_declaration" | "var_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.utf8_text(content.as_bytes()).ok() == Some(var_name)
                        && pos_ok(name_node, before)
                    {
                        let ty = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                            .map(|s| s.to_string());
                        return Some((ty, node_start_position(name_node)));
                    }
                }
            }
            "parameter" | "class_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.utf8_text(content.as_bytes()).ok() == Some(var_name) {
                        let ty = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                            .map(|s| s.to_string());
                        return Some((ty, node_start_position(name_node)));
                    }
                }
            }
            "parameters" | "class_parameters" => {
                if let Some(found) = scan_node_for_declaration(child, content, var_name, before) {
                    return Some(found);
                }
            }
            "enumerator" => {
                // `for { x <- xs }` — first named identifier binds.
                let mut ec = child.walk();
                let id = child.children(&mut ec).find(|c| c.kind() == "identifier");
                if let Some(id_node) = id {
                    if id_node.utf8_text(content.as_bytes()).ok() == Some(var_name) {
                        return Some((None, node_start_position(id_node)));
                    }
                }
            }
            "case_clause" => {
                // `case y: Int =>` — pattern binds `y` with type `Int`.
                let mut cc = child.walk();
                for pat in child.children(&mut cc) {
                    if pat.kind() == "typed_pattern" {
                        let mut tpc = pat.walk();
                        let kids: Vec<Node> = pat.children(&mut tpc).filter(|n| n.is_named()).collect();
                        let id = kids.iter().find(|n| n.kind() == "identifier");
                        let ty = kids.iter().find(|n| n.kind() == "type_identifier" || n.kind() == "generic_type");
                        if let (Some(id_n), Some(ty_n)) = (id, ty) {
                            if id_n.utf8_text(content.as_bytes()).ok() == Some(var_name) {
                                let ty_text = ty_n
                                    .utf8_text(content.as_bytes())
                                    .ok()
                                    .map(|s| s.to_string());
                                return Some((ty_text, node_start_position(*id_n)));
                            }
                        }
                    }
                }
            }
            // Recurse into scope-introducing nodes.
            "block" | "template_body" | "indented_block" | "function_definition"
            | "function_declaration" | "match_expression" | "for_expression"
            | "if_expression" | "while_expression" | "do_while_expression"
            | "try_expression" | "case_block" | "lambda_expression"
            | "extension_definition" => {
                if let Some(found) = scan_node_for_declaration(child, content, var_name, before) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn pos_ok(node: Node, before: Option<&Position>) -> bool {
    let Some(p) = before else { return true; };
    let np = node.start_position();
    np.row < p.line as usize || (np.row == p.line as usize && np.column <= p.character as usize)
}

fn node_start_position(node: Node) -> Position {
    Position {
        line: node.start_position().row as u32,
        character: node.start_position().column as u32,
    }
}

/// Collect every visible val/var/parameter/enumerator binding under `scope`
/// whose identifier start position is at or before `position`.  Used by
/// `find_declarations_in_scope`.
fn collect_scope_declarations(
    scope: Node,
    content: &str,
    position: &Position,
    out: &mut Vec<(String, Option<String>)>,
    seen: &mut HashSet<String>,
) {
    let mut cursor = scope.walk();
    for child in scope.children(&mut cursor) {
        match child.kind() {
            "val_definition" | "var_definition" => {
                if let Some(name_node) = child.child_by_field_name("pattern") {
                    if name_node.kind() == "identifier" && pos_ok(name_node, Some(position)) {
                        push_unique(name_node, content, child.child_by_field_name("type"), out, seen);
                    }
                }
            }
            "val_declaration" | "var_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if pos_ok(name_node, Some(position)) {
                        push_unique(name_node, content, child.child_by_field_name("type"), out, seen);
                    }
                }
            }
            "parameter" | "class_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    push_unique(name_node, content, child.child_by_field_name("type"), out, seen);
                }
            }
            "parameters" | "class_parameters" => {
                collect_scope_declarations(child, content, position, out, seen);
            }
            _ => {}
        }
    }
}

fn push_unique(
    name_node: Node,
    content: &str,
    type_node: Option<Node>,
    out: &mut Vec<(String, Option<String>)>,
    seen: &mut HashSet<String>,
) {
    let Ok(name) = name_node.utf8_text(content.as_bytes()) else {
        return;
    };
    if seen.insert(name.to_string()) {
        let ty = type_node.and_then(|n| n.utf8_text(content.as_bytes()).ok().map(|s| s.to_string()));
        out.push((name.to_string(), ty));
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

    // --- Position / type resolution (Phase 3).

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        // Tree built so we can reuse get_literal_type / find_variable_type.
        let temp = self.parse_str(content)?;
        let leaf = innermost_named_node(node, position)?;
        // Literal → primitive type.
        if let Some(t) = literal_type_for(&leaf) {
            return Some(t);
        }
        // Identifier → walk up for context-specific resolution.
        if leaf.kind() == "identifier" || leaf.kind() == "type_identifier" {
            let name = leaf.utf8_text(content.as_bytes()).ok()?;
            return self.find_variable_type(&temp.0, content, name, position);
        }
        None
    }

    fn find_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let node = get_node_at_position(tree, content, position)?;
        let leaf = innermost_named_node(node, position)?;
        if leaf.kind() != "identifier" && leaf.kind() != "type_identifier" {
            return None;
        }
        let name = leaf.utf8_text(content.as_bytes()).ok()?.to_string();
        // If parent is a field_expression and the leaf is the trailing name,
        // emit the receiver chain as the qualifier.
        let qualifier = leaf.parent().and_then(|p| {
            if p.kind() != "field_expression" {
                return None;
            }
            // field_expression children: <receiver> `.` <name>.  Identify the
            // receiver as the first named child; only emit a qualifier when the
            // selected leaf is the trailing identifier.
            let mut c = p.walk();
            let kids: Vec<Node> = p.children(&mut c).filter(|n| n.is_named()).collect();
            if kids.len() < 2 {
                return None;
            }
            let last = kids.last()?;
            if last.id() != leaf.id() {
                return None;
            }
            let receiver = kids.first()?;
            receiver
                .utf8_text(content.as_bytes())
                .ok()
                .map(|s| s.to_string())
        });
        Some((name, qualifier))
    }

    fn find_variable_type(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<String> {
        find_variable_declaration_impl(tree, content, var_name, position)
            .and_then(|(ty, _)| ty)
    }

    fn find_variable_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(Option<String>, Position)> {
        find_variable_declaration_impl(tree, content, var_name, position)
    }

    fn find_declarations_in_scope(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Vec<(String, Option<String>)> {
        let Some(node) = get_node_at_position(tree, content, position) else {
            return Vec::new();
        };
        let mut out: Vec<(String, Option<String>)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut cursor = Some(node);
        while let Some(n) = cursor {
            collect_scope_declarations(n, content, position, &mut out, &mut seen);
            cursor = n.parent();
        }
        out
    }

    fn extract_call_arguments(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<Vec<(String, Position)>> {
        let node = get_node_at_position(tree, content, position)?;
        let call = nearest_ancestor(node, "call_expression")?;
        let args_node = call
            .child_by_field_name("arguments")
            .or_else(|| first_child_with_kind(call, "arguments"))?;
        let mut c = args_node.walk();
        let mut out = Vec::new();
        for child in args_node.children(&mut c) {
            // Skip punctuation: `(`, `,`, `)`.
            if !child.is_named() {
                continue;
            }
            let text = child
                .utf8_text(content.as_bytes())
                .ok()
                .map(|s| s.to_string())?;
            let pos = Position {
                line: child.start_position().row as u32,
                character: child.start_position().column as u32,
            };
            out.push((text, pos));
        }
        Some(out)
    }

    fn get_literal_type(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        let node = get_node_at_position(tree, content, position)?;
        let leaf = innermost_named_node(node, position)?;
        literal_type_for(&leaf)
    }

    fn get_method_receiver_and_params(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<(String, Vec<String>)> {
        let leaf = innermost_named_node(node, position)?;
        let call = nearest_ancestor(leaf, "call_expression")?;
        let fn_node = call.child_by_field_name("function").or_else(|| {
            // Grammars without a `function` field: first named child is the callee.
            let mut c = call.walk();
            call.children(&mut c).find(|n| n.is_named() && n.kind() != "arguments")
        })?;
        let receiver = match fn_node.kind() {
            "field_expression" => {
                let mut c = fn_node.walk();
                let kids: Vec<Node> = fn_node.children(&mut c).filter(|n| n.is_named()).collect();
                kids.first()?
                    .utf8_text(content.as_bytes())
                    .ok()?
                    .to_string()
            }
            _ => fn_node
                .utf8_text(content.as_bytes())
                .ok()?
                .to_string(),
        };
        let args_node = call
            .child_by_field_name("arguments")
            .or_else(|| first_child_with_kind(call, "arguments"));
        let mut params = Vec::new();
        if let Some(args) = args_node {
            let mut c = args.walk();
            for child in args.children(&mut c) {
                if !child.is_named() {
                    continue;
                }
                if let Ok(t) = child.utf8_text(content.as_bytes()) {
                    params.push(t.to_string());
                }
            }
        }
        Some((receiver, params))
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
