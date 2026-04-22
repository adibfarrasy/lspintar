//! Shared algorithm for finding all identifier occurrences that resolve to a
//! given local variable or parameter declaration, honouring lexical scope and
//! shadowing.

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Tree};

use crate::ts_helper::get_node_at_position;

/// Find every identifier occurrence in `tree` that resolves to the declaration
/// at `decl_position`.  The declaration's own ident range is included.
///
/// `decl_node_kinds` lists tree-sitter node kinds whose descendant `identifier`
/// nodes are *declarations* (binding introductions).  An occurrence of `name`
/// beneath a declaration node of the same name other than the target is
/// treated as a shadow: the subtree rooted at that declaration node is
/// skipped.
///
/// `scope_node_kinds` lists tree-sitter node kinds that bound the search.  The
/// search starts at the closest enclosing scope ancestor of the declaration.
pub fn find_local_references(
    tree: &Tree,
    content: &str,
    decl_position: &Position,
    decl_node_kinds: &[&str],
    scope_node_kinds: &[&str],
) -> Option<Vec<Range>> {
    let decl_node = get_node_at_position(tree, content, decl_position)?;
    let name = decl_node.utf8_text(content.as_bytes()).ok()?.to_string();
    if name.is_empty() {
        return None;
    }

    // Walk up to the enclosing declaration node so we can identify the
    // original binding, then up again to the scope that contains it.
    let binding_node = ancestor_of_kinds(decl_node, decl_node_kinds).unwrap_or(decl_node);
    let scope = ancestor_of_kinds(binding_node, scope_node_kinds)
        .or_else(|| binding_node.parent())
        .unwrap_or(binding_node);

    let bytes = content.as_bytes();
    let mut out: Vec<Range> = Vec::new();
    collect_refs(
        scope,
        bytes,
        &name,
        binding_node,
        decl_node_kinds,
        &mut out,
    );
    Some(out)
}

fn ancestor_of_kinds<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if kinds.contains(&n.kind()) {
            return Some(n);
        }
        cur = n.parent();
    }
    None
}

fn collect_refs(
    node: Node,
    bytes: &[u8],
    name: &str,
    original_binding: Node,
    decl_node_kinds: &[&str],
    out: &mut Vec<Range>,
) {
    // If this node is a declaration node other than the original, and it
    // introduces a binding with the same name, skip its entire subtree
    // (shadow).
    if decl_node_kinds.contains(&node.kind()) && node.id() != original_binding.id() {
        if declares_name(node, bytes, name) {
            return;
        }
    }

    if node.kind() == "identifier" || node.kind() == "simple_identifier" {
        if let Ok(text) = node.utf8_text(bytes) {
            if text == name && !is_member_access_rhs(node) && !is_label_or_type_context(node) {
                out.push(node_to_range(&node));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_refs(child, bytes, name, original_binding, decl_node_kinds, out);
    }
}

/// True when a declaration node introduces a binding whose name is `name`.
/// Scans immediate-ish descendants rather than full subtree to avoid matching
/// initializer expressions.
fn declares_name(node: Node, bytes: &[u8], name: &str) -> bool {
    // For most decl kinds the name is either:
    //   - a child `name` field (formal_parameter)
    //   - a descendant `variable_declarator.name`
    //   - an `identifier`/`simple_identifier` child
    if let Some(n) = node.child_by_field_name("name") {
        if n.utf8_text(bytes).map(|t| t == name).unwrap_or(false) {
            return true;
        }
    }
    // Fallback: first identifier child at any depth within declarators.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
            if child.utf8_text(bytes).map(|t| t == name).unwrap_or(false) {
                return true;
            }
        }
        if child.kind() == "variable_declarator" || child.kind() == "variable_declaration" {
            if let Some(n) = child.child_by_field_name("name") {
                if n.utf8_text(bytes).map(|t| t == name).unwrap_or(false) {
                    return true;
                }
            }
            // variable_declarator without a `name` field: check first ident
            let mut inner = child.walk();
            for ic in child.children(&mut inner) {
                if (ic.kind() == "identifier" || ic.kind() == "simple_identifier")
                    && ic.utf8_text(bytes).map(|t| t == name).unwrap_or(false)
                {
                    return true;
                }
                if ic.kind() != "identifier"
                    && ic.kind() != "simple_identifier"
                {
                    // stop at first non-identifier sibling (initializer)
                    break;
                }
            }
        }
    }
    false
}

/// True when this identifier is the member name on the RHS of a `.` access
/// (e.g. `obj.name` — the `name` is not the local `name`).  These contexts
/// must not be counted as references to the local.
fn is_member_access_rhs(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "field_access" | "member_access_expression" | "navigation_expression"
        | "navigation_suffix" => {
            // The identifier is the RHS when it's the `field`/`name` child.
            if let Some(field) = parent.child_by_field_name("field").or_else(|| parent.child_by_field_name("name")) {
                return field.id() == node.id();
            }
            // Kotlin navigation_suffix / navigation_expression: identifier
            // after the dot is always the member side.
            let mut cursor = parent.walk();
            let mut saw_dot = false;
            for child in parent.children(&mut cursor) {
                if child.kind() == "." {
                    saw_dot = true;
                    continue;
                }
                if saw_dot && child.id() == node.id() {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// True when this identifier is in a context that is a label or a type name
/// rather than an expression reference — and therefore cannot be a local
/// variable reference.
fn is_label_or_type_context(node: Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    matches!(
        parent.kind(),
        "type_identifier"
            | "generic_type"
            | "scoped_type_identifier"
            | "annotation"
            | "marker_annotation"
            | "label"
            | "break_statement"
            | "continue_statement"
    )
}

fn node_to_range(node: &Node) -> Range {
    Range {
        start: Position {
            line: node.start_position().row as u32,
            character: node.start_position().column as u32,
        },
        end: Position {
            line: node.end_position().row as u32,
            character: node.end_position().column as u32,
        },
    }
}
