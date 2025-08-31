use std::{fs::read_to_string, path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    core::{
        constants::JAVA_PARSER,
        dependency_cache::DependencyCache,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, get_language_support_for_file,
            node_to_lsp_location, uri_to_path, uri_to_tree,
        },
    },
    languages::{java::constants::JAVA_COMMON_IMPORTS, LanguageSupport},
};

use super::method_resolution::{extract_call_signature_from_context, find_method_with_signature};

/// Get or create a compiled query for Java
pub fn get_or_create_query(query_text: &str) -> Result<Query, tree_sitter::QueryError> {
    let language = JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into());
    Query::new(language, query_text)
}

#[tracing::instrument(skip_all)]
pub fn get_declaration_query_for_symbol_type(symbol_type: &SymbolType) -> Option<&'static str> {
    match symbol_type {
        SymbolType::Type => Some(
            r#"
            (class_declaration name: (identifier) @name)
            (interface_declaration name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
            (annotation_type_declaration name: (identifier) @name)
        "#,
        ),
        SymbolType::SuperClass => Some(r#"(class_declaration name: (identifier) @name)"#),
        SymbolType::SuperInterface => Some(r#"(interface_declaration name: (identifier) @name)"#),
        SymbolType::MethodCall => Some(r#"(method_declaration name: (identifier) @name)"#),
        SymbolType::FieldUsage => Some(
            r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        ),
        SymbolType::VariableUsage => Some(
            r#"
            (local_variable_declaration declarator: (variable_declarator name: (identifier) @name))
            (formal_parameter name: (identifier) @name)
            (field_declaration declarator: (variable_declarator name: (identifier) @name))
        "#,
        ),
        _ => None,
    }
}

#[tracing::instrument(skip_all)]
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

        // Early termination for single-result queries (local scope)
        if !candidates.is_empty()
            && is_local_scope_query(query_text)
            && !query_text.contains("local_variable_declaration")
        {
            break;
        }
    }

    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

/// Check if this is a query that should terminate early for local scope
fn is_local_scope_query(query_text: &str) -> bool {
    query_text.contains("formal_parameter") || query_text.contains("local_variable_declaration")
}

#[tracing::instrument(skip_all)]
pub fn search_definition<'a>(tree: &'a Tree, source: &str, symbol_name: &str) -> Option<Node<'a>> {
    // Try different declaration types for Java
    let queries = [
        r#"(class_declaration name: (identifier) @name)"#,
        r#"(interface_declaration name: (identifier) @name)"#,
        r#"(enum_declaration name: (identifier) @name)"#,
        r#"(annotation_type_declaration name: (identifier) @name)"#,
        r#"(method_declaration name: (identifier) @name)"#,
        r#"(field_declaration declarator: (variable_declarator name: (identifier) @name))"#,
        r#"(constructor_declaration name: (identifier) @name)"#,
    ];

    for query_text in &queries {
        if let Some(candidates) = find_definition_candidates(tree, source, symbol_name, query_text)
        {
            if let Some(first_candidate) = candidates.first() {
                return Some(*first_candidate);
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
pub fn search_definition_in_project(
    origin_file_uri: &str,
    origin_source: &str,
    usage_node: &Node,
    target_file_uri: &str,
    _language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(origin_source.as_bytes()).ok()?;
    let origin_tree = uri_to_tree(origin_file_uri)?;

    // Get the appropriate language support for the origin file (where the symbol usage is)
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

#[tracing::instrument(skip_all)]
pub fn prepare_symbol_lookup_key_with_wildcard_support(
    usage_node: &Node,
    source: &str,
    file_uri: &str,
    project_root: Option<PathBuf>,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<(PathBuf, String)> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    use tracing::debug;

    let project_root = project_root.or_else(|| {
        uri_to_path(file_uri).and_then(|path| {
            find_project_root(&path).or_else(|| find_external_dependency_root(&path))
        })
    });

    let project_root = match project_root {
        Some(root) => root,
        None => {
            return None;
        }
    };

    // First try direct symbol lookup
    let direct_key = (project_root.clone(), symbol_name.clone());
    // Check using read-through cache pattern
    if dependency_cache
        .find_symbol_sync(&direct_key.0, &direct_key.1)
        .is_some()
    {
        return Some(direct_key);
    }

    // Debug: show comprehensive cache information

    // Show all project roots in cache
    let _project_roots: std::collections::HashSet<_> = dependency_cache
        .symbol_index
        .iter()
        .map(|entry| entry.key().0.clone())
        .collect();

    // Show symbols for current project
    if !dependency_cache.symbol_index.is_empty() {
        let _current_project_keys: Vec<_> = dependency_cache
            .symbol_index
            .iter()
            .filter(|entry| entry.key().0 == project_root)
            .map(|entry| entry.key().1.clone())
            .collect();
    }

    // Try to resolve through imports
    let imports = extract_imports_from_source(source);

    // Check explicit imports first
    for import in &imports {
        if import.ends_with(&format!(".{}", symbol_name)) {
            let explicit_key = (project_root.clone(), import.clone());

            // Check using read-through cache pattern
            // First try current project (symbols and builtins)
            if dependency_cache
                .find_symbol_sync(&explicit_key.0, &explicit_key.1)
                .is_some()
                || dependency_cache.find_builtin_info(import).is_some()
            {
                return Some(explicit_key.clone());
            }

            // Then try current project's external dependencies (JAR files)
            if let Some(_) = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    dependency_cache
                        .find_project_external_info(&project_root, import)
                        .await
                })
            }) {
                return Some(explicit_key);
            }

            // Then try dependency projects
            if let Some(project_metadata) = dependency_cache.project_metadata.get(&project_root) {
                for dependent_project_ref in project_metadata.inter_project_deps.iter() {
                    let dependent_project = dependent_project_ref.clone();
                    let dep_key = (dependent_project.clone(), import.clone());

                    if dependency_cache
                        .find_symbol_sync(&dep_key.0, &dep_key.1)
                        .is_some()
                        || dependency_cache.find_builtin_info(import).is_some()
                    {
                        return Some(dep_key);
                    }

                    // Also check external dependencies of this dependency project
                    if let Some(_) = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            dependency_cache
                                .find_project_external_info(&dependent_project, import)
                                .await
                        })
                    }) {
                        return Some(dep_key);
                    }
                }
            }

            debug!(
                "Java utils: Explicit import '{}' not found in current project or dependencies",
                import
            );
        } else {
            debug!(
                "Java utils: import '{}' does not match symbol '{}'",
                import, symbol_name
            );
        }
    }

    // Try wildcard imports
    let wildcard_imports = get_wildcard_imports_from_source(source);
    for package in wildcard_imports {
        let wildcard_key = (project_root.clone(), format!("{}.{}", package, symbol_name));
        // Check using read-through cache pattern
        if dependency_cache
            .find_symbol_sync(&wildcard_key.0, &wildcard_key.1)
            .is_some()
            || dependency_cache
                .find_builtin_info(&wildcard_key.1)
                .is_some()
        {
            return Some(wildcard_key);
        }
    }

    // Try same package (default package or current package)
    if let Some(current_package) = extract_package_from_source(source) {
        let same_package_key = (
            project_root.clone(),
            format!("{}.{}", current_package, symbol_name),
        );
        // Check using read-through cache pattern
        if dependency_cache
            .find_symbol_sync(&same_package_key.0, &same_package_key.1)
            .is_some()
            || dependency_cache
                .find_builtin_info(&same_package_key.1)
                .is_some()
        {
            return Some(same_package_key);
        }
    }

    let packages = JAVA_COMMON_IMPORTS
        .iter()
        .map(|import| import.strip_suffix(".*").unwrap_or(import))
        .collect::<Vec<&str>>();

    for package in &packages {
        let java_key = (project_root.clone(), format!("{}.{}", package, symbol_name));
        // Check using read-through cache pattern
        if dependency_cache
            .find_symbol_sync(&java_key.0, &java_key.1)
            .is_some()
            || dependency_cache.find_builtin_info(&java_key.1).is_some()
        {
            return Some(java_key);
        }
    }

    // Last resort: original symbol name
    debug!("Java utils: No matches found, returning original symbol name");
    let result = Some((project_root.clone(), symbol_name.clone()));
    result
}

pub fn extract_imports_from_source(source: &str) -> Vec<String> {
    let mut imports = Vec::new();

    if let Ok(query) = get_or_create_query(r#"(import_declaration (scoped_identifier) @import)"#) {
        let language = JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into());
        let mut parser = Parser::new();
        if parser.set_language(language).is_ok() {
            if let Some(tree) = parser.parse(source, None) {
                let mut cursor = QueryCursor::new();
                cursor
                    .matches(&query, tree.root_node(), source.as_bytes())
                    .for_each(|m| {
                        for capture in m.captures {
                            if let Ok(import_text) = capture.node.utf8_text(source.as_bytes()) {
                                imports.push(import_text.to_string());
                            }
                        }
                    });
            }
        }
    }

    imports
}

pub fn get_wildcard_imports_from_source(source: &str) -> Vec<String> {
    let mut wildcard_imports = Vec::new();

    // Look for import statements ending with .*
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") && trimmed.ends_with(".*;") {
            let mut package = trimmed[7..trimmed.len() - 3].trim();

            // Remove "static" keyword if present
            if package.starts_with("static ") {
                package = &package[7..];
            }

            wildcard_imports.push(package.to_string());
        }
    }

    wildcard_imports
}

pub fn extract_package_from_source(source: &str) -> Option<String> {
    if let Ok(query) = get_or_create_query(r#"(package_declaration (scoped_identifier) @package)"#)
    {
        let language = JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into());
        let mut parser = Parser::new();
        if parser.set_language(language).is_ok() {
            if let Some(tree) = parser.parse(source, None) {
                let mut cursor = QueryCursor::new();
                let mut result = None;

                cursor
                    .matches(&query, tree.root_node(), source.as_bytes())
                    .for_each(|m| {
                        for capture in m.captures {
                            if let Ok(package_text) = capture.node.utf8_text(source.as_bytes()) {
                                result = Some(package_text.to_string());
                            }
                        }
                    });

                return result;
            }
        }
    }

    None
}

/// Resolve symbol name with import context
#[tracing::instrument(skip_all)]
pub fn resolve_symbol_with_imports(
    symbol_name: &str,
    source: &str,
    dependency_cache: &Arc<DependencyCache>,
) -> Option<String> {
    use tracing::debug;

    let imports = extract_imports_from_source(source);

    // First, check for exact matches and specific imports
    let mut star_imports = Vec::new();
    for import in &imports {
        let expected_suffix = format!(".{}", symbol_name);
        let matches_suffix = import.ends_with(&expected_suffix);
        let exact_match = import == symbol_name;

        if matches_suffix || exact_match {
            debug!(
                "Java resolve_symbol_with_imports: found exact match import {}",
                import
            );
            return Some(import.clone());
        }

        // Collect star imports for later use
        if import.ends_with(".*") {
            let package = import.strip_suffix(".*").unwrap_or("");
            star_imports.push(package);
        }
    }

    // For common Java types, try java.lang first (always implicitly imported)
    let common_java_types = [
        "String",
        "Integer",
        "Long",
        "Double",
        "Float",
        "Boolean",
        "Character",
        "Byte",
        "Short",
        "Object",
        "Class",
        "System",
        "Math",
        "Thread",
        "Runnable",
        "Exception",
        "RuntimeException",
        "Error",
        "Throwable",
        "Number",
        "Comparable",
        "Cloneable",
        "Serializable",
        "Iterable",
        "Collection",
        "List",
        "Set",
        "Map",
        "ArrayList",
        "HashMap",
        "HashSet",
        "LinkedList",
        "TreeMap",
        "TreeSet",
        "Queue",
        "Deque",
        "Stack",
        "Vector",
    ];

    if common_java_types.contains(&symbol_name.as_ref()) {
        let java_lang_fqn = format!("java.lang.{}", symbol_name);
        debug!(
            "Java resolve_symbol_with_imports: using java.lang for common type: {}",
            java_lang_fqn
        );
        return Some(java_lang_fqn);
    }

    // Try star imports
    for package in star_imports {
        let candidate_fqn = format!("{}.{}", package, symbol_name);
        debug!(
            "Java resolve_symbol_with_imports: trying star import: {}",
            candidate_fqn
        );
        
        // Verify this FQN exists in cache/database
        if verify_fqn_exists(&candidate_fqn, dependency_cache) {
            return Some(candidate_fqn);
        }
    }

    debug!(
        "Java resolve_symbol_with_imports: no resolution found for {}",
        symbol_name
    );
    None
}

/// Verify that a given FQN exists in the dependency cache or workspace
fn verify_fqn_exists(fqn: &str, dependency_cache: &Arc<DependencyCache>) -> bool {
    // Check builtin classes (like java.lang.* classes)
    if let Some(class_name) = fqn.split('.').last() {
        if dependency_cache.builtin_infos.get(class_name).is_some() {
            return true;
        }
    }

    // Check if the FQN exists anywhere in the symbol index
    for entry in dependency_cache.symbol_index.iter() {
        let ((_project_root, symbol_name), _file_path) = (entry.key(), entry.value());
        if symbol_name == fqn {
            return true;
        }
    }

    false
}
