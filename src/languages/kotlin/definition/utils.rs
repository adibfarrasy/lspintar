use std::{fs::read_to_string, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        constants::KOTLIN_PARSER,
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{
            find_project_root, get_language_support_for_file, node_to_lsp_location,
            set_start_position_for_language, uri_to_path, uri_to_tree,
        },
    },
    languages::LanguageSupport,
};

use super::method_resolution::{
    extract_call_signature_from_context, find_method_with_signature, CallSignature,
};

pub fn set_start_position(source: &str, usage_node: &Node, file_uri: &str) -> Option<Location> {
    set_start_position_for_language(source, usage_node, file_uri, "kotlin")
}

/// Get or create a compiled query for Kotlin
pub fn get_or_create_query(query_text: &str) -> Result<Query, tree_sitter::QueryError> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    Query::new(language, query_text)
}

/// Get the appropriate tree-sitter query for a given symbol type
pub fn get_declaration_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration (type_identifier) @name)
            (interface_declaration (type_identifier) @name)
            (object_declaration (type_identifier) @name)
            (enum_declaration (type_identifier) @name)
            (annotation_declaration (type_identifier) @name)
            (type_alias (type_identifier) @name)
        "#,
        ),
        SymbolType::SuperClass => Some(r#"(class_declaration (type_identifier) @name)"#),
        SymbolType::SuperInterface => Some(r#"(interface_declaration (type_identifier) @name)"#),
        SymbolType::MethodCall => Some(r#"(function_declaration (simple_identifier) @name)"#),
        SymbolType::FieldUsage => {
            Some(r#"(property_declaration (variable_declaration (simple_identifier) @name))"#)
        }
        SymbolType::VariableUsage => Some(
            r#"
            (property_declaration (variable_declaration (simple_identifier) @name))
            (parameter (simple_identifier) @name)
            (class_parameter (simple_identifier) @name)
            (lambda_parameter (simple_identifier) @name)
        "#,
        ),
        _ => None,
    }
}

/// Search for definition candidates using tree-sitter queries
pub fn find_definition_candidates<'a>(
    tree: &'a Tree,
    source: &str,
    symbol_name: &str,
    query_text: &str,
) -> Option<Vec<Node<'a>>> {
    let query = get_or_create_query(query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                if node_text == symbol_name {
                    candidates.push(capture.node);
                }
            }
        }
    }

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

/// Search for a definition in a tree using multiple query strategies
pub fn search_definition<'a>(tree: &'a Tree, source: &str, symbol_name: &str) -> Option<Node<'a>> {
    // Try different declaration types for Kotlin
    let queries = [
        r#"(function_declaration (simple_identifier) @name)"#,
        r#"(class_declaration (type_identifier) @name)"#,
        r#"(class_declaration (modifiers) (type_identifier) @name)"#,
        r#"(interface_declaration (type_identifier) @name)"#,
        r#"(interface_declaration (modifiers) (type_identifier) @name)"#,
        r#"(object_declaration (type_identifier) @name)"#,
        r#"(object_declaration (modifiers) (type_identifier) @name)"#,
        r#"(enum_declaration (type_identifier) @name)"#,
        r#"(enum_declaration (modifiers) (type_identifier) @name)"#,
        r#"(type_alias (type_identifier) @name)"#,
        r#"(type_alias (modifiers) (type_identifier) @name)"#,
        r#"(property_declaration (variable_declaration (simple_identifier) @name))"#,
        r#"(parameter (simple_identifier) @name)"#,
        r#"(class_parameter (simple_identifier) @name)"#,
    ];

    for query_text in &queries {
        if let Some(candidates) = find_definition_candidates(tree, source, symbol_name, query_text)
        {
            if let Some(first_candidate) = candidates.first() {
                // Find the actual declaration node (parent of the identifier)
                return find_declaration_parent(*first_candidate);
            }
        }
    }

    None
}

/// Find the declaration node that contains the identifier
fn find_declaration_parent(identifier_node: Node) -> Option<Node> {
    let mut current = identifier_node;

    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "object_declaration"
            | "enum_declaration"
            | "type_alias"
            | "property_declaration"
            | "parameter"
            | "class_parameter" => {
                return Some(parent);
            }
            _ => current = parent,
        }
    }

    Some(identifier_node)
}

/// Extract package name from Kotlin source code
pub fn extract_package_from_source(source: &str) -> Option<String> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;

    let tree = parser.parse(source, None)?;
    
    let query_text = r#"(package_header (identifier) @package)"#;
    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                return Some(text.to_string());
            }
        }
    }

    None
}

/// Extract import statements from Kotlin source
pub fn extract_imports_from_source(source: &str) -> Vec<String> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let root = tree.root_node();
    let mut imports = Vec::new();

    // Look for import_list and import_header nodes
    for child in root.children(&mut root.walk()) {
        if child.kind() == "import_list" {
            // Look for import_header nodes within import_list
            for import_child in child.children(&mut child.walk()) {
                if import_child.kind() == "import_header" {
                    if let Ok(import_text) = import_child.utf8_text(source.as_bytes()) {
                        // Extract the import path (remove "import " prefix)
                        if let Some(import_path) =
                            import_text.strip_prefix("import ").map(|s| s.trim())
                        {
                            imports.push(import_path.to_string());
                        }
                    }
                }
            }
        } else if child.kind() == "import_header" {
            // Direct import_header node
            if let Ok(import_text) = child.utf8_text(source.as_bytes()) {
                // Extract the import path (remove "import " prefix)
                if let Some(import_path) = import_text.strip_prefix("import ").map(|s| s.trim()) {
                    imports.push(import_path.to_string());
                }
            }
        }
    }

    imports
}

/// Resolve symbol name with import context
#[tracing::instrument(skip_all)]
pub fn resolve_symbol_with_imports(
    symbol_name: &str,
    source: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<String> {
    let imports = extract_imports_from_source(source);

    // First, check for exact matches and specific imports
    let mut star_imports = Vec::new();

    for import in &imports {
        let expected_suffix = format!(".{}", symbol_name);
        let matches_suffix = import.ends_with(&expected_suffix);
        let exact_match = import == symbol_name;

        if matches_suffix || exact_match {
            debug!("found exact match import {}", import);
            return Some(import.clone());
        }

        // Collect star imports for later use
        if import.ends_with(".*") {
            let package = import.strip_suffix(".*").unwrap_or("");
            star_imports.push(package);
        }
    }

    // For basic Kotlin types, try common kotlin stdlib patterns FIRST
    // This prevents incorrect resolution to current package or star imports
    let common_kotlin_types = [
        "String",
        "Int",
        "Long",
        "Double",
        "Float",
        "Boolean",
        "Char",
        "Byte",
        "Short",
        "Any",
        "Unit",
        "Nothing",
        "List",
        "MutableList",
        "Set",
        "MutableSet",
        "Map",
        "MutableMap",
        "Collection",
        "MutableCollection",
        "Array",
        "BooleanArray",
        "ByteArray",
        "CharArray",
        "DoubleArray",
        "FloatArray",
        "IntArray",
        "LongArray",
        "ShortArray",
    ];
    if common_kotlin_types.contains(&symbol_name.as_ref()) {
        // Collection types are in kotlin.collections, others are in kotlin
        let kotlin_stdlib_candidates = if [
            "List",
            "MutableList",
            "Set",
            "MutableSet",
            "Map",
            "MutableMap",
            "Collection",
            "MutableCollection",
            "Array",
            "BooleanArray",
            "ByteArray",
            "CharArray",
            "DoubleArray",
            "FloatArray",
            "IntArray",
            "LongArray",
            "ShortArray",
        ]
        .contains(&symbol_name.as_ref())
        {
            [
                format!("commonMain.kotlin.collections.{}", symbol_name),
                format!("jvmMain.kotlin.collections.{}", symbol_name),
                format!("kotlin.collections.{}", symbol_name),
            ]
        } else {
            [
                format!("commonMain.kotlin.{}", symbol_name),
                format!("jvmMain.kotlin.{}", symbol_name),
                format!("kotlin.{}", symbol_name),
            ]
        };

        for candidate in &kotlin_stdlib_candidates {
            if dependency_cache.find_builtin_info(candidate).is_some() {
                return Some(candidate.clone());
            }
        }

        // For basic types, if not found in builtins, just return the simple name
        // The external dependency system should find it
        return Some(symbol_name.to_string());
    }

    // Try with current package (but only for non-basic types)
    if let Some(package) = extract_package_from_source(source) {
        let _result = format!("{}.{}", package, symbol_name);
        // Only return if we can verify it exists - but for now just fall through
    }

    // Use star imports as fallback
    if !star_imports.is_empty() {
        // For services, prefer service packages over param packages
        let preferred_package = if symbol_name.ends_with("Service") {
            star_imports
                .iter()
                .find(|p| p.ends_with(".service") || p.contains(".service."))
                .or_else(|| star_imports.first())
        } else {
            star_imports.first()
        };

        if let Some(package) = preferred_package {
            let result = format!("{}.{}", package, symbol_name);
            return Some(result);
        }
    }

    // Fallback to symbol name alone
    Some(symbol_name.to_string())
}

/// Prepare symbol lookup key with wildcard and import support
pub fn prepare_symbol_lookup_key_with_wildcard_support(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    _call_signature: Option<CallSignature>,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<(PathBuf, String)> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let project_root = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;

    // Try to resolve the symbol with import context
    let fqn = resolve_symbol_with_imports(&symbol_name, source, dependency_cache)?;

    Some((project_root, fqn))
}

/// Search for definition in a specific project
pub fn search_definition_in_project(
    origin_file_uri: &str,
    origin_source: &str,
    usage_node: &Node,
    target_file_uri: &str,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(origin_source.as_bytes()).ok()?;
    let origin_tree = uri_to_tree(origin_file_uri)?;

    let origin_file_path = uri_to_path(origin_file_uri)?;
    let origin_language_support = get_language_support_for_file(&origin_file_path)?;

    let symbol_type = origin_language_support
        .determine_symbol_type_from_context(&origin_tree, usage_node, origin_source)
        .ok()?;

    let target_tree = uri_to_tree(target_file_uri)?;
    let target_source = read_to_string(uri_to_path(target_file_uri)?).ok()?;

    // For method calls, use enhanced method resolution
    if symbol_type == SymbolType::MethodCall {
        if let Some(call_signature) = extract_call_signature_from_context(usage_node, origin_source)
        {
            if let Some(method_node) = find_method_with_signature(
                &target_tree,
                &target_source,
                &symbol_name,
                &call_signature,
            ) {
                return node_to_lsp_location(&method_node, target_file_uri);
            }
        }
    }

    // Fallback to general definition search
    let definition_node = search_definition(&target_tree, &target_source, &symbol_name)?;
    node_to_lsp_location(&definition_node, target_file_uri)
}
