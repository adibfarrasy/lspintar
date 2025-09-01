use std::{path::PathBuf, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{
        constants::IS_INDEXING_COMPLETED,
        dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
        jar_utils::get_uri,
        state_manager::get_global,
        utils::{
            find_external_dependency_root, find_project_root, node_to_lsp_location, uri_to_path,
        },
    },
};

use super::utils::prepare_symbol_lookup_key_with_wildcard_support;

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
        if parent.kind() == "navigation_expression" {
            if let Some(enum_type_node) = parent.child(0) {
                if let Some(enum_type_name) = super::project::resolve_nested_enum_type(source, &enum_type_node) {
                    if enum_type_name.contains('.') {
                        // Note: Using a dummy language_support for external dependencies
                        return super::project::find_nested_enum_using_regular_resolution(
                            source,
                            file_uri,
                            &enum_type_name,
                            symbol_text,
                            dependency_cache.clone(),
                            &crate::languages::kotlin::KotlinSupport,
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
    if !get_global(IS_INDEXING_COMPLETED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false) {
        return None;
    }

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

    // For core Kotlin types, check builtins first to avoid triggering decompilation
    // when source files exist in Kotlin standard library
    if is_core_kotlin_type(&resolved_symbol) {
        // Try multiple Kotlin builtin candidates
        let builtin_candidates = vec![
            resolved_symbol.clone(),
            format!("commonMain.kotlin.{}", resolved_symbol),
            format!("jvmMain.kotlin.{}", resolved_symbol),
            format!("kotlin.{}", resolved_symbol),
        ];
        
        for candidate in &builtin_candidates {
            if let Some(builtin_info) = dependency_cache.find_builtin_info(candidate) {
                return search_external_definition_and_convert(&symbol_name, builtin_info);
            }
        }
    }

    // First try current project  
    // For basic types and collections, also try with kotlin prefix
    let common_kotlin_types = ["String", "Int", "Long", "Double", "Float", "Boolean", "Char", "Byte", "Short", "Any", "Unit", "Nothing", "List", "MutableList", "Set", "MutableSet", "Map", "MutableMap", "Collection", "MutableCollection", "Array", "BooleanArray", "ByteArray", "CharArray", "DoubleArray", "FloatArray", "IntArray", "LongArray", "ShortArray"];
    let collection_types = ["List", "MutableList", "Set", "MutableSet", "Map", "MutableMap", "Collection", "MutableCollection", "Array", "BooleanArray", "ByteArray", "CharArray", "DoubleArray", "FloatArray", "IntArray", "LongArray", "ShortArray"];
    
    let kotlin_candidates = if common_kotlin_types.contains(&resolved_symbol.as_str()) {
        if collection_types.contains(&resolved_symbol.as_str()) {
            vec![
                resolved_symbol.clone(),
                format!("commonMain.kotlin.collections.{}", resolved_symbol),
                format!("jvmMain.kotlin.collections.{}", resolved_symbol),
                format!("kotlin.collections.{}", resolved_symbol),
            ]
        } else {
            vec![
                resolved_symbol.clone(),
                format!("commonMain.kotlin.{}", resolved_symbol),
                format!("jvmMain.kotlin.{}", resolved_symbol),
                format!("kotlin.{}", resolved_symbol),
            ]
        }
    } else {
        vec![resolved_symbol.clone()]
    };
    
    for candidate in &kotlin_candidates {
        // First try symbol index (for source files like .kt)
        if let Some(symbol_path) = dependency_cache
            .find_symbol(&current_project, candidate)
            .await
        {
            let source_info = SourceFileInfo::new(symbol_path, None, None);
            return search_external_definition_and_convert(&symbol_name, source_info);
        }
        
        // Then try external info (for decompiled .class files)
        if let Some(source_info) = dependency_cache
            .find_external_symbol_with_lazy_parsing(&current_project, candidate)
            .await
        {
            return search_external_definition_and_convert(&symbol_name, source_info);
        }
    }

    // Then try projects this project depends on (using project_metadata)
    if let Some(project_metadata) = dependency_cache.project_metadata.get(&current_project) {
        for dependent_project_ref in project_metadata.inter_project_deps.iter() {
            let dependent_project = dependent_project_ref.clone();
            if let Some(source_info) = dependency_cache
                .find_external_symbol_with_lazy_parsing(&dependent_project, &resolved_symbol)
                .await
            {
                return search_external_definition_and_convert(&symbol_name, source_info);
            }

            // Also check if the symbol exists directly in the dependency project (not as external dependency)
            if let Some(symbol_path) = dependency_cache
                .find_symbol(&dependent_project, &resolved_symbol)
                .await
            {
                // Convert to external source info format
                let source_info = SourceFileInfo::new(symbol_path, None, None);
                return search_external_definition_and_convert(&symbol_name, source_info);
            }
        }
    }

    // Also try external dependency project roots
    let _temp_dir_prefix = "lspintar_builtin_sources";
    let mut external_project_roots = std::collections::HashSet::new();
    
    // Collect all external dependency project roots from the symbol index
    for entry in dependency_cache.symbol_index.iter() {
        let (project_root, _) = entry.key();
        if crate::core::utils::is_external_dependency(project_root) {
            external_project_roots.insert(project_root.clone());
            tracing::debug!("Found external dependency project root: {:?}", project_root);
        }
    }
    
    tracing::debug!("Found {} external dependency project roots", external_project_roots.len());
    
    // Search in external dependency project roots
    for external_root in external_project_roots {
        tracing::debug!("Searching in external root: {:?}", external_root);
        for candidate in &kotlin_candidates {
            tracing::debug!("Trying candidate '{}' in external root {:?}", candidate, external_root);
            if let Some(symbol_path) = dependency_cache
                .find_symbol(&external_root, candidate)
                .await
            {
                tracing::debug!("Found symbol '{}' at path {:?}", candidate, symbol_path);
                let source_info = SourceFileInfo::new(symbol_path, None, None);
                return search_external_definition_and_convert(&symbol_name, source_info);
            }
        }
    }

    // Fallback: try builtin sources (Kotlin standard library, etc.)
    for candidate in &kotlin_candidates {
        if let Some(builtin_info) = dependency_cache.find_builtin_info(candidate) {
            return search_external_definition_and_convert(&symbol_name, builtin_info);
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn search_external_definition_and_convert(
    symbol_name: &str,
    source_info: SourceFileInfo,
) -> Option<Location> {
    let tree = source_info.get_tree().ok()?;
    let content = source_info.get_content().ok()?;
    
    let definition_node = {
        // For decompiled .class files, use Java language support instead of current language
        if source_info.zip_internal_path.as_ref().map_or(false, |p| p.ends_with(".class")) {
            use crate::languages::java::definition::utils::search_definition as java_search_definition;
            java_search_definition(&tree, &content, symbol_name)?
        } else {
            use super::utils::search_definition;
            search_definition(&tree, &content, symbol_name)?
        }
    };
    
    let file_uri = get_uri(&source_info)?;
    node_to_lsp_location(&definition_node, &file_uri)
}

/// Check if a type is a core Kotlin type that should prioritize builtin sources
/// over JAR dependencies to avoid unnecessary decompilation
#[tracing::instrument(skip_all)]
fn is_core_kotlin_type(type_name: &str) -> bool {
    const CORE_KOTLIN_TYPES: &[&str] = &[
        "String", "Int", "Long", "Double", "Float", "Boolean", "Char", "Byte", "Short",
        "Any", "Unit", "Nothing",
        "List", "MutableList", "Set", "MutableSet", "Map", "MutableMap",
        "Collection", "MutableCollection",
        "Array", "BooleanArray", "ByteArray", "CharArray", "DoubleArray", 
        "FloatArray", "IntArray", "LongArray", "ShortArray",
    ];
    
    CORE_KOTLIN_TYPES.contains(&type_name)
}

