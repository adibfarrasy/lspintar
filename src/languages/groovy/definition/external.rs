use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{
        constants::IS_INDEXING_COMPLETED,
        dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
        jar_utils::get_uri,
        state_manager::get_global,
        symbols::SymbolType,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
        },
    },
    lsp_warning,
};

use super::utils::{prepare_symbol_lookup_key_with_wildcard_support, search_definition};

#[tracing::instrument(skip_all)]
pub async fn find_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    
    // FIRST: Check for nested enum access patterns (same as find_in_project)
    
    if let Some(parent) = usage_node.parent() {
        if parent.kind() == "field_access" {
            if let Some(enum_type_node) = parent.child_by_field_name("object") {
                if let Some(enum_type_name) = super::project::resolve_nested_enum_type(source, &enum_type_node) {
                    if enum_type_name.contains('.') {
                        // Note: Using a dummy language_support for external dependencies
                        return super::project::find_nested_enum_using_regular_resolution(
                            source,
                            file_uri,
                            &enum_type_name,
                            symbol_text,
                            dependency_cache.clone(),
                            &crate::languages::groovy::GroovySupport,
                        ).await;
                    }
                }
            }
        }
    }

    let current_project = uri_to_path(file_uri).and_then(|path| {
        find_project_root(&path).or_else(|| find_external_dependency_root(&path))
    })?;

    find_project_external(
        source,
        file_uri,
        usage_node,
        current_project,
        dependency_cache.clone(),
    )
    .await
}

#[tracing::instrument(skip_all)]
async fn find_project_external(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    current_project: PathBuf,
    dependency_cache: Arc<DependencyCache>,
) -> Option<Location> {
    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();

    // Try to resolve the symbol through imports (including wildcard imports)
    let resolved_symbol = if let Some((_, fully_qualified_name)) =
        prepare_symbol_lookup_key_with_wildcard_support(
            usage_node,
            source,
            file_uri,
            None,
            &dependency_cache,
        ) {
        // Use the full qualified name for external lookup (not just the class name)
        fully_qualified_name
    } else {
        symbol_name.clone()
    };

    // For core Groovy/Java classes, check builtins first to avoid triggering decompilation
    // when source files exist in JAVA_HOME/src.zip or Groovy standard library
    if is_core_groovy_or_java_class(&resolved_symbol) {
        if let Some(builtin_info) = dependency_cache.find_builtin_info(&resolved_symbol) {
            return search_external_definition_and_convert(&symbol_name, builtin_info);
        }
    }

    // First try current project
    if let Some(external_info) = dependency_cache
        .find_external_symbol_with_lazy_parsing(&current_project, &resolved_symbol)
        .await
    {
        return search_external_definition_and_convert(&symbol_name, external_info);
    }

    // Then try projects this project depends on (using project_metadata)
    if let Some(project_metadata) = dependency_cache.project_metadata.get(&current_project) {
        for dependent_project_ref in project_metadata.inter_project_deps.iter() {
            let dependent_project = dependent_project_ref.clone();
            if let Some(external_info) = dependency_cache
                .find_external_symbol_with_lazy_parsing(&dependent_project, &resolved_symbol)
                .await
            {
                return search_external_definition_and_convert(&symbol_name, external_info);
            }

            // Also check if the symbol exists directly in the dependency project (not as external dependency)
            if let Some(symbol_path) = dependency_cache
                .find_symbol(&dependent_project, &resolved_symbol)
                .await
            {
                // Convert to external source info format
                let external_info = SourceFileInfo::new(symbol_path, None, None);
                return search_external_definition_and_convert(&symbol_name, external_info);
            } else {
            }
        }
    } else {

        // Fallback: try all other projects in the cache (as before)
        let mut checked_projects = std::collections::HashSet::new();
        checked_projects.insert(current_project.clone());

        for entry in dependency_cache.project_external_infos.iter() {
            let (project_root, _) = entry.key();
            if !checked_projects.contains(project_root) {
                checked_projects.insert(project_root.clone());
                if let Some(external_info) = dependency_cache
                    .find_external_symbol_with_lazy_parsing(project_root, &resolved_symbol)
                    .await
                {
                    return search_external_definition_and_convert(&symbol_name, external_info);
                }
            }
        }
    }

    if let Some(external_info) = dependency_cache.find_builtin_info(&resolved_symbol) {
        return search_external_definition_and_convert(&symbol_name, external_info);
    }

    if get_global(IS_INDEXING_COMPLETED).is_none() {
        lsp_warning!("Indexing still in progress...");
    }

    None
}

#[tracing::instrument(skip_all)]
fn search_external_definition_and_convert(
    symbol_name: &str,
    external_info: SourceFileInfo,
) -> Option<Location> {
    let tree = external_info
        .get_tree()
        .context(format!("failed to get tree for {symbol_name}"))
        .ok()?;

    let content = external_info
        .get_content()
        .context(format!("failed to get content for {symbol_name}"))
        .ok()?;

    let definition_node = {
        // For decompiled .class files, use Java language support instead of Groovy
        if external_info.zip_internal_path.as_ref().map_or(false, |p| p.ends_with(".class")) {
            use crate::languages::java::definition::utils::search_definition as java_search_definition;
            java_search_definition(&tree, &content, symbol_name)?
        } else {
            search_definition(&tree, &content, symbol_name, SymbolType::Type)
                .context(format!("definition for {symbol_name} not found"))
                .ok()?
        }
    };

    let file_uri = get_uri(&external_info.clone())
        .context(format!("file_uri for {symbol_name} not found"))
        .ok()?;

    node_to_lsp_location(&definition_node, &file_uri)
}


/// Check if a class is a core Groovy or Java class that should prioritize builtin sources
/// over JAR dependencies to avoid unnecessary decompilation
#[tracing::instrument(skip_all)]
fn is_core_groovy_or_java_class(fully_qualified_name: &str) -> bool {
    use crate::languages::groovy::constants::GROOVY_DEFAULT_IMPORTS;
    
    // Convert import patterns to prefixes for matching
    GROOVY_DEFAULT_IMPORTS.iter().any(|import| {
        if import == &"groovy.*" {
            // Special case: groovy.* matches groovy.lang., groovy.util., etc.
            fully_qualified_name.starts_with("groovy.")
        } else if import.ends_with(".*") {
            // Wildcard import: java.math.* matches java.math.BigDecimal
            let prefix = import.strip_suffix(".*").unwrap();
            fully_qualified_name.starts_with(&format!("{}.", prefix))
        } else {
            // Exact import: java.math.BigDecimal matches java.math.BigDecimal
            fully_qualified_name == *import
        }
    })
}
