use lsp_core::{
    language_support::{
        CallArgData, ClassDeclarationData, GenericTypeUsage, IdentResult, LanguageSupport,
        MemberAccessData, MethodCallSiteData, MethodSig, ObjectCreationData,
        OverrideMethodData, ParameterResult, ParseResult,
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
/// Kind alone — caller should prefer `literal_type_with_source` when source
/// is available so suffix-tagged forms (`1L`, `1.0f`) refine to the precise
/// primitive.
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

/// Like [`literal_type_for`] but inspects the source text of `node` to
/// refine integer / floating-point literals based on Scala's suffix
/// conventions: `1L`/`1l` → Long, `1.0f`/`1.0F` → Float, `1.0d`/`1.0D` →
/// Double (the default).  Falls back to the kind-only classification when
/// no recognised suffix is present.
fn literal_type_with_source(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "integer_literal" => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            if text.ends_with('L') || text.ends_with('l') {
                Some("Long".to_string())
            } else {
                Some("Int".to_string())
            }
        }
        "floating_point_literal" => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            if text.ends_with('f') || text.ends_with('F') {
                Some("Float".to_string())
            } else {
                Some("Double".to_string())
            }
        }
        _ => literal_type_for(node),
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

/// Smallest enclosing block-like node where a local binding lives.
fn enclosing_scope<'a>(node: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.parent();
    while let Some(n) = cursor {
        match n.kind() {
            "block"
            | "indented_block"
            | "template_body"
            | "function_definition"
            | "function_declaration"
            | "lambda_expression"
            | "for_expression"
            | "match_expression"
            | "case_block"
            | "case_clause"
            | "extension_definition"
            | "compilation_unit" => return Some(n),
            _ => cursor = n.parent(),
        }
    }
    None
}

/// Walk every identifier descendant of `scope` whose text equals `name`.
/// Skips identifiers that live inside a nested scope which itself
/// re-declares `name` (lexical shadowing).
fn collect_local_refs(
    node: Node,
    root_scope_id: usize,
    source: &str,
    name: &str,
    out: &mut Vec<Range>,
) {
    let introduces_scope = matches!(
        node.kind(),
        "block"
            | "indented_block"
            | "lambda_expression"
            | "function_definition"
            | "function_declaration"
            | "case_clause"
            | "for_expression"
    );
    if introduces_scope && node.id() != root_scope_id && scope_shadows(node, source, name) {
        return;
    }
    if node.kind() == "identifier"
        && node.utf8_text(source.as_bytes()).ok() == Some(name)
    {
        out.push(node_to_range(node));
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        collect_local_refs(child, root_scope_id, source, name, out);
    }
}

/// True when the direct children of `scope` introduce a binding named
/// `name` (val/var/parameter/enumerator/case_clause typed_pattern).
fn scope_shadows(scope: Node, source: &str, name: &str) -> bool {
    let mut c = scope.walk();
    for child in scope.children(&mut c) {
        let id_match = |n: Node| -> bool {
            n.utf8_text(source.as_bytes()).ok() == Some(name)
        };
        match child.kind() {
            "val_definition" | "var_definition" => {
                if let Some(p) = child.child_by_field_name("pattern") {
                    if p.kind() == "identifier" && id_match(p) {
                        return true;
                    }
                }
            }
            "val_declaration" | "var_declaration" => {
                if let Some(n) = child.child_by_field_name("name") {
                    if id_match(n) {
                        return true;
                    }
                }
            }
            "parameter" | "class_parameter" => {
                if let Some(n) = child.child_by_field_name("name") {
                    if id_match(n) {
                        return true;
                    }
                }
            }
            "parameters" | "class_parameters" => {
                if scope_shadows(child, source, name) {
                    return true;
                }
            }
            "enumerator" => {
                let mut ec = child.walk();
                if let Some(id) = child.children(&mut ec).find(|c| c.kind() == "identifier") {
                    if id_match(id) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn node_to_range(n: Node) -> Range {
    Range {
        start: Position {
            line: n.start_position().row as u32,
            character: n.start_position().column as u32,
        },
        end: Position {
            line: n.end_position().row as u32,
            character: n.end_position().column as u32,
        },
    }
}

/// Walk every named node in the subtree rooted at `start`, applying `f` to each.
fn walk_named<F: FnMut(Node)>(start: Node, f: &mut F) {
    if start.is_named() {
        f(start);
    }
    let mut cursor = start.walk();
    for child in start.children(&mut cursor) {
        walk_named(child, f);
    }
}

/// Walk class/object/trait/enum definitions and apply `f` to every direct
/// method (`function_definition` / `function_declaration`) in their body.
fn walk_class_methods<F: FnMut(&str, Node)>(start: Node, source: &str, f: &mut F) {
    walk_named(start, &mut |n| {
        let is_class_like = matches!(
            n.kind(),
            "class_definition" | "object_definition" | "trait_definition" | "enum_definition"
        );
        if !is_class_like {
            return;
        }
        let Some(name_node) = n.child_by_field_name("name") else { return };
        let Ok(class_name) = name_node.utf8_text(source.as_bytes()) else { return };
        let Some(body) = n.child_by_field_name("body") else { return };
        let mut c = body.walk();
        for child in body.children(&mut c) {
            if matches!(child.kind(), "function_definition" | "function_declaration") {
                f(class_name, child);
            }
        }
    });
}

/// True when `node`'s `modifiers` block (or a sibling keyword token) contains
/// the literal modifier text `name`.
fn modifier_present(node: Node, source: &str, name: &str) -> bool {
    let mut c = node.walk();
    for child in node.children(&mut c) {
        match child.kind() {
            "modifiers" => {
                let mut mc = child.walk();
                for m in child.children(&mut mc) {
                    let text = m.utf8_text(source.as_bytes()).unwrap_or("").trim();
                    if text == name || m.kind() == name {
                        return true;
                    }
                }
            }
            k if k == name => return true,
            _ => {}
        }
    }
    false
}

/// Reduce a possibly-qualified type node (`type_identifier`,
/// `generic_type`, `stable_type_identifier`) to its short type name and the
/// range of the identifier token where diagnostics should be anchored.
fn short_type_name_and_range(n: Node, source: &str) -> (String, Range) {
    match n.kind() {
        "type_identifier" => {
            let text = n
                .utf8_text(source.as_bytes())
                .map(|s| s.to_string())
                .unwrap_or_default();
            (text, node_to_range(n))
        }
        "generic_type" => {
            if let Some(head) = n
                .child_by_field_name("type")
                .or_else(|| first_child_with_kind(n, "type_identifier"))
                .or_else(|| first_child_with_kind(n, "stable_type_identifier"))
            {
                return short_type_name_and_range(head, source);
            }
            (String::new(), node_to_range(n))
        }
        "stable_type_identifier" => {
            // Children: stable_identifier (qualifier), `.`, type_identifier.  The
            // trailing type_identifier is the short name.
            let mut c = n.walk();
            let mut last_id: Option<Node> = None;
            for child in n.children(&mut c) {
                if child.kind() == "type_identifier" {
                    last_id = Some(child);
                }
            }
            if let Some(id) = last_id {
                return (
                    id.utf8_text(source.as_bytes())
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    node_to_range(id),
                );
            }
            (String::new(), node_to_range(n))
        }
        _ => {
            let text = n
                .utf8_text(source.as_bytes())
                .map(|s| s.to_string())
                .unwrap_or_default();
            (text, node_to_range(n))
        }
    }
}

/// For a `function_definition` or `function_declaration`, return its short
/// name (`Some`) and the textual parameter types (one per positional param,
/// flattened across multiple parameter lists).
fn method_sig_components(func: Node, source: &str) -> (Option<String>, Vec<String>) {
    let name = func
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()));
    let mut params = Vec::new();
    let mut c = func.walk();
    for child in func.children(&mut c) {
        if child.kind() == "parameters" {
            let mut pc = child.walk();
            for p in child.children(&mut pc) {
                if p.kind() == "parameter" || p.kind() == "class_parameter" {
                    let ty = p
                        .child_by_field_name("type")
                        .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()))
                        .unwrap_or_default();
                    params.push(ty);
                }
            }
        }
    }
    (name, params)
}

/// For a `call_expression` whose function is `<receiver>.<method>` and whose
/// receiver is a simple `identifier`, return the four pieces needed for both
/// `get_member_accesses` and `get_method_call_sites`.
fn extract_simple_member_access(
    call: Node,
    source: &str,
) -> Option<(String, Range, String, Range)> {
    let fn_node = call.child_by_field_name("function").or_else(|| {
        let mut c = call.walk();
        call.children(&mut c)
            .find(|n| n.is_named() && n.kind() != "arguments")
    })?;
    if fn_node.kind() != "field_expression" {
        return None;
    }
    let mut c = fn_node.walk();
    let kids: Vec<Node> = fn_node.children(&mut c).filter(|n| n.is_named()).collect();
    if kids.len() < 2 {
        return None;
    }
    let receiver = kids.first()?;
    let member = kids.last()?;
    if receiver.kind() != "identifier" {
        return None;
    }
    let receiver_text = receiver.utf8_text(source.as_bytes()).ok()?.to_string();
    let member_text = member.utf8_text(source.as_bytes()).ok()?.to_string();
    Some((
        receiver_text,
        node_to_range(*receiver),
        member_text,
        node_to_range(*member),
    ))
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
            "simple_enum_case" => node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier")?,
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
            // Scala 3 enum cases — each `case X` behaves like a public
            // static final field on the enum companion.  Treat as Field
            // so symbol search can find them.
            "simple_enum_case" => Some(NodeKind::Field),
            "function_definition" | "function_declaration" => Some(NodeKind::Function),
            "val_definition" | "var_definition" | "val_declaration" | "var_declaration"
            | "given_definition" => Some(NodeKind::Field),
            "type_definition" => Some(NodeKind::Class),
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        // Scala 3 enum cases bind a name via a bare `identifier` child
        // (`simple_enum_case` -> `identifier "Red"`), not via any of the
        // field-name patterns covered by GET_VAL_SHORT_NAME_QUERY.
        if node.kind() == "simple_enum_case" {
            let id = node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier")?;
            return id.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
        }
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
        // Literal → primitive type (suffix-aware).
        if let Some(t) = literal_type_with_source(&leaf, content) {
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
        literal_type_with_source(&leaf, content)
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

    fn find_local_references(
        &self,
        tree: &Tree,
        content: &str,
        decl_position: &Position,
    ) -> Option<Vec<Range>> {
        // Locate the declaration identifier at `decl_position`, then walk
        // its enclosing scope collecting every `identifier` occurrence whose
        // text matches.  Identifiers inside a nested scope that re-declares
        // the same name are excluded (lexical shadowing).
        let raw = get_node_at_position(tree, content, decl_position)?;
        let decl_id = if raw.is_named() && raw.kind() == "identifier" {
            raw
        } else {
            innermost_named_node(raw, decl_position)?
        };
        if decl_id.kind() != "identifier" {
            return None;
        }
        let name = decl_id.utf8_text(content.as_bytes()).ok()?;
        let scope = enclosing_scope(decl_id)?;
        let scope_id = scope.id();
        let mut out = Vec::new();
        collect_local_refs(scope, scope_id, content, name, &mut out);
        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    // --- Phase 4: diagnostics data ---

    fn get_type_references(&self, tree: &Tree, source: &str) -> Vec<(String, Range)> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            if n.kind() == "type_identifier" {
                if let Ok(t) = n.utf8_text(source.as_bytes()) {
                    out.push((t.to_string(), node_to_range(n)));
                }
            }
        });
        out
    }

    fn get_declared_type_names(&self, tree: &Tree, source: &str) -> Vec<String> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            let name_node = match n.kind() {
                "class_definition" | "object_definition" | "trait_definition"
                | "enum_definition" => n.child_by_field_name("name"),
                "type_definition" => n.child_by_field_name("name"),
                _ => None,
            };
            if let Some(name_node) = name_node {
                if let Ok(t) = name_node.utf8_text(source.as_bytes()) {
                    out.push(t.to_string());
                }
            }
        });
        out
    }

    fn get_class_declarations(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<ClassDeclarationData> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            let is_class_like = matches!(
                n.kind(),
                "class_definition" | "trait_definition" | "enum_definition"
            );
            if !is_class_like {
                return;
            }
            let Some(name_node) = n.child_by_field_name("name") else {
                return;
            };
            let Ok(name) = name_node.utf8_text(source.as_bytes()) else {
                return;
            };
            let ident_range = node_to_range(name_node);
            // Abstract when explicitly modified `abstract`, or for traits which
            // are implicitly abstract.
            let is_abstract = n.kind() == "trait_definition"
                || modifier_present(n, source, "abstract");
            let parents = n
                .child_by_field_name("extend")
                .map(|ext| extends_clause_types(ext, source))
                .unwrap_or_default();
            // Collect direct method signatures from the body.
            let mut defined_methods = Vec::new();
            if let Some(body) = n.child_by_field_name("body") {
                let mut c = body.walk();
                for child in body.children(&mut c) {
                    if matches!(child.kind(), "function_definition" | "function_declaration") {
                        if let (Some(mname), params) = method_sig_components(child, source) {
                            defined_methods.push(MethodSig::new(mname, params));
                        }
                    }
                }
            }
            out.push(ClassDeclarationData {
                name: name.to_string(),
                ident_range,
                is_abstract,
                parents,
                defined_methods,
            });
        });
        out
    }

    fn get_object_creations(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<ObjectCreationData> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            if n.kind() != "instance_expression" {
                return;
            }
            // First named child after `new` is the instantiated type.
            let mut c = n.walk();
            let typed = n
                .children(&mut c)
                .find(|child| child.is_named() && child.kind() != "arguments");
            let Some(t) = typed else { return };
            let (short, range) = short_type_name_and_range(t, source);
            if !short.is_empty() {
                out.push(ObjectCreationData { type_name: short, range });
            }
        });
        out
    }

    fn get_member_accesses(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<MemberAccessData> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            if n.kind() != "call_expression" {
                return;
            }
            let Some((receiver_name, receiver_range, member_name, member_range)) =
                extract_simple_member_access(n, source)
            else {
                return;
            };
            out.push(MemberAccessData {
                receiver_name,
                member_name,
                member_range,
                receiver_range,
            });
        });
        out
    }

    fn get_generic_type_usages(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<GenericTypeUsage> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            if n.kind() != "generic_type" {
                return;
            }
            // Base type name: drill into the head (type_identifier or
            // stable_type_identifier), keeping only the short name.
            let head = n.child_by_field_name("type").or_else(|| {
                let mut c = n.walk();
                n.children(&mut c).find(|c| c.is_named() && c.kind() != "type_arguments")
            });
            let Some(head) = head else { return };
            let (short, _) = short_type_name_and_range(head, source);
            if short.is_empty() {
                return;
            }
            let args = n
                .child_by_field_name("type_arguments")
                .or_else(|| first_child_with_kind(n, "type_arguments"));
            let Some(args) = args else { return };
            let mut c = args.walk();
            let arg_count = args
                .children(&mut c)
                .filter(|c| c.is_named())
                .count();
            out.push(GenericTypeUsage {
                type_name: short,
                arg_count,
                range: node_to_range(n),
            });
        });
        out
    }

    fn get_override_methods(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<OverrideMethodData> {
        let mut out = Vec::new();
        walk_class_methods(tree.root_node(), source, &mut |class_name, func| {
            if !modifier_present(func, source, "override") {
                return;
            }
            let Some(name_node) = func.child_by_field_name("name") else { return };
            let Ok(method_name) = name_node.utf8_text(source.as_bytes()) else { return };
            let return_type = func
                .child_by_field_name("return_type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()));
            out.push(OverrideMethodData {
                containing_class: class_name.to_string(),
                method_name: method_name.to_string(),
                return_type,
                range: node_to_range(name_node),
            });
        });
        out
    }

    fn get_method_call_sites(
        &self,
        tree: &Tree,
        source: &str,
    ) -> Vec<MethodCallSiteData> {
        let mut out = Vec::new();
        walk_named(tree.root_node(), &mut |n| {
            if n.kind() != "call_expression" {
                return;
            }
            let Some((receiver_name, receiver_range, method_name, method_range)) =
                extract_simple_member_access(n, source)
            else {
                return;
            };
            let args_node = n
                .child_by_field_name("arguments")
                .or_else(|| first_child_with_kind(n, "arguments"));
            let mut args = Vec::new();
            if let Some(args_node) = args_node {
                let mut c = args_node.walk();
                for child in args_node.children(&mut c) {
                    if !child.is_named() {
                        continue;
                    }
                    let text = child
                        .utf8_text(source.as_bytes())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    args.push(CallArgData {
                        node_kind: child.kind().to_string(),
                        text,
                        range: node_to_range(child),
                    });
                }
            }
            out.push(MethodCallSiteData {
                receiver_name,
                receiver_range,
                method_name,
                method_range,
                args,
            });
        });
        out
    }
}
