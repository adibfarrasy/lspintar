use tree_sitter::Node;

/// Calculate the scope distance between a usage node and a declaration node.
/// Returns None if the declaration is not in scope of the usage.
/// Lower numbers indicate closer scopes (higher priority).
pub fn calculate_scope_distance(usage_node: &Node, declaration_node: &Node) -> Option<usize> {
    // Check if declaration is in scope of usage
    if !is_in_scope(usage_node, declaration_node) {
        return None;
    }

    // Calculate nesting distance
    let usage_depth = get_nesting_depth(usage_node);
    let decl_depth = get_nesting_depth(declaration_node);

    // Prefer closer scopes (higher depth difference means closer)
    Some(usage_depth.saturating_sub(decl_depth))
}

/// Check if a declaration is in scope of a usage node
pub fn is_in_scope(usage_node: &Node, declaration_node: &Node) -> bool {
    let decl_method = find_containing_method(declaration_node);
    let usage_method = find_containing_method(usage_node);
    let decl_block = find_containing_block(declaration_node);
    let usage_block = find_containing_block(usage_node);
    
    // For formal parameters, check if usage is in the same method
    if let Some(decl_method) = decl_method {
        if let Some(usage_method) = usage_method {
            return decl_method.id() == usage_method.id();
        }
    }

    // For local variables, check if declaration comes before usage in same block
    if let Some(decl_block) = decl_block {
        if let Some(usage_block) = usage_block {
            if decl_block.id() == usage_block.id() {
                return declaration_node.start_position() < usage_node.start_position();
            }
        }
    }

    // Handle top-level declarations: if declaration has no containing block,
    // it's accessible from any nested scope as long as it comes before usage
    if decl_block.is_none() {
        // Declaration is at top level, check if it comes before usage
        if declaration_node.start_position() < usage_node.start_position() {
            // Additional check: make sure they're in the same top-level context
            if let Some(usage_method) = usage_method {
                // Usage is inside a method, declaration should be either:
                // 1. A parameter of the same method, or
                // 2. A top-level declaration accessible to that method
                if let Some(decl_method) = find_containing_method(declaration_node) {
                    // Both are in methods - must be same method for parameters
                    return decl_method.id() == usage_method.id();
                } else {
                    // Declaration is at class/file level, usage is in method - accessible
                    return true;
                }
            } else {
                // Both are at the same level (class/file level)
                return true;
            }
        }
    }

    false
}

/// Find the containing method of a node
pub fn find_containing_method<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "method_declaration" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Find the containing block of a node
pub fn find_containing_block<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "block" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Get the nesting depth of a node (count of enclosing blocks/methods/classes)
pub fn get_nesting_depth(node: &Node) -> usize {
    let mut depth = 0;
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "block" | "method_declaration" | "class_declaration"
        ) {
            depth += 1;
        }
        current = parent.parent();
    }
    depth
}