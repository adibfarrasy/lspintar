use std::{fs::read_to_string, sync::Arc};

use tower_lsp::lsp_types::Location;
use tree_sitter::Node;

use crate::{
    core::{dependency_cache::DependencyCache, utils::{node_to_lsp_location, uri_to_tree, path_to_file_uri, find_project_root, uri_to_path}}, 
    languages::LanguageSupport
};

use super::utils::{resolve_symbol_with_imports, search_definition, extract_imports_from_source};

pub async fn find_in_workspace(
    source: &str,
    file_uri: &str,
    usage_node: &Node<'_>,
    dependency_cache: Arc<DependencyCache>,
    language_support: &dyn LanguageSupport,
) -> Option<Location> {
    let symbol_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
    
    // FIRST: Check for nested enum access patterns (same as find_in_project)
    if let Some(parent) = usage_node.parent() {
        if parent.kind() == "navigation_expression" {
            if let Some(enum_type_node) = parent.child(0) {
                if let Some(enum_type_name) = super::project::resolve_nested_enum_type(source, &enum_type_node) {
                    if enum_type_name.contains('.') {
                        return super::project::find_nested_enum_using_regular_resolution(
                            source,
                            file_uri,
                            &enum_type_name,
                            symbol_text,
                            dependency_cache.clone(),
                            language_support,
                        ).await;
                    }
                }
            }
        }
    }

    let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let _current_project = uri_to_path(file_uri).and_then(|path| find_project_root(&path))?;
    
    // Try to resolve the symbol with import context
    let fqn = resolve_symbol_with_imports(&symbol_name, source, &dependency_cache)?;
    
    // Extract imports for additional search patterns
    let imports = extract_imports_from_source(source);
    let mut search_keys = vec![fqn.clone()];
    
    // Add fully qualified import names that match our symbol
    for import in &imports {
        if import.ends_with(&format!(".{}", symbol_name)) {
            search_keys.push(import.clone());
        }
    }
    
    // Search across all projects in the workspace
    for search_fqn in search_keys {
        // Search through the symbol index
        for entry in dependency_cache.symbol_index.iter() {
            let ((_entry_project_root, entry_fqn), file_path) = (entry.key(), entry.value());
            
            // Check if this entry might contain our symbol
            if entry_fqn == &search_fqn || 
               entry_fqn.ends_with(&format!(".{}", search_fqn)) ||
               search_fqn.ends_with(&format!(".{}", entry_fqn)) {
                
                // Read the file and search for the definition
                if let Ok(content) = read_to_string(&file_path) {
                    let file_uri_string = path_to_file_uri(&file_path)?;
                    
                    if let Some(tree) = uri_to_tree(&file_uri_string) {
                        // Extract just the class name for searching
                        let class_name = search_fqn.split('.').last().unwrap_or(&symbol_name);
                        
                        // Try searching for the class name first
                        if let Some(definition_node) = search_definition(&tree, &content, class_name) {
                            if let Some(location) = node_to_lsp_location(&definition_node, &file_uri_string) {
                                return Some(location);
                            }
                        }
                        
                        // Fallback to searching for the symbol name directly if different
                        if class_name != symbol_name {
                            if let Some(definition_node) = search_definition(&tree, &content, &symbol_name) {
                                if let Some(location) = node_to_lsp_location(&definition_node, &file_uri_string) {
                                    return Some(location);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    None
}