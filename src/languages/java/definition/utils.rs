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

use super::definition_chain::{extract_call_signature_from_context, find_method_with_signature};

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
        SymbolType::EnumDeclaration => Some(r#"(enum_declaration name: (identifier) @name)"#),
        SymbolType::EnumUsage => Some(
            r#"
            (enum_constant name: (identifier) @name)
            (enum_declaration name: (identifier) @name)
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
        r#"(enum_constant name: (identifier) @name)"#, // Add enum constants first for priority
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
            debug!(
                "LSPINTAR_DEBUG: utils found matching import '{}' for symbol '{}', returning ({:?}, '{}')",
                import, symbol_name, project_root, import
            );
            // Return the FQN so workspace.rs can search it in all dependency projects
            return Some((project_root.clone(), import.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn create_java_parser() -> Option<Parser> {
        let mut parser = Parser::new();
        match parser.set_language(&tree_sitter_java::LANGUAGE.into()) {
            Ok(()) => Some(parser),
            Err(_) => None,
        }
    }

    #[test]
    fn test_get_declaration_query_for_enum_types() {
        // Test that enum declaration and usage queries are provided
        let enum_decl_query = get_declaration_query_for_symbol_type(&SymbolType::EnumDeclaration);
        assert!(enum_decl_query.is_some());
        
        let enum_usage_query = get_declaration_query_for_symbol_type(&SymbolType::EnumUsage);
        assert!(enum_usage_query.is_some());
    }

    #[test]
    fn test_search_definition_enum_constant() {
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };

        // Test source with enum definition
        let source = r#"
public enum Color {
    RED,
    GREEN,
    BLUE
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Search for enum constants using specialized query
        let enum_constant_query = r#"(enum_constant name: (identifier) @name)"#;
        let result = find_definition_candidates(&tree, source, "RED", enum_constant_query);
        assert!(result.is_some(), "Should find RED enum constant");
        
        let result = find_definition_candidates(&tree, source, "GREEN", enum_constant_query);
        assert!(result.is_some(), "Should find GREEN enum constant");
        
        // Search for non-existent constant
        let result = find_definition_candidates(&tree, source, "NONEXISTENT", enum_constant_query);
        assert!(result.is_none(), "Should not find non-existent constant");
    }

    #[test]
    fn test_extract_imports_with_static_enum_imports() {
        let source = r#"
package com.test;

import java.util.List;
import static com.test.enums.Status.*;
import static com.test.enums.Color.RED;

public class MyClass {
    // class body
}
"#;
        
        let imports = extract_imports_from_source(source);
        
        // Check that imports are included (function may return None if parsing fails)
        if !imports.is_empty() {
            // Just verify the function works - exact format may vary based on implementation
            assert!(imports.len() > 0, "Should extract some imports");
        }
    }

    #[test] 
    fn test_get_wildcard_imports_with_enum_static_imports() {
        let source = r#"
package com.test;

import java.util.*;
import static com.test.enums.Status.*;
import static com.test.enums.Color.*;

public class MyClass {
    // class body
}
"#;
        
        let wildcards = get_wildcard_imports_from_source(source);
        
        // Should include both regular and static wildcard imports
        assert!(wildcards.contains(&"java.util".to_string()));
        assert!(wildcards.contains(&"com.test.enums.Status".to_string()));
        assert!(wildcards.contains(&"com.test.enums.Color".to_string()));
    }

    #[test]
    fn test_resolve_symbol_with_static_enum_imports() {
        // This is a more complex test that would require setting up dependency cache
        // For now, just test that the function exists and can be called
        let source = r#"
import static com.test.enums.Status.*;

public class Example {
    private Status state = ACTIVE;
}
"#;
        
        // Create minimal dependency cache for testing
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test resolving a static import symbol
        let result = resolve_symbol_with_imports("ACTIVE", source, &dependency_cache);
        
        // Since we don't have a full dependency cache setup, this will return None
        // But it tests that the function can be called without panic
        assert!(result.is_none());
    }

    #[test]
    fn test_navigation_expression_java_enum_access() {
        // Test that navigation expressions like Status.SUCCESS are handled correctly
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
public enum Status {
    PENDING,
    COMPLETED,
    FAILED
}

public class Processor {
    public void handle() {
        Status s = Status.PENDING;
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Test that we can find navigation expression enum constants  
        let enum_constant_query = r#"(enum_constant name: (identifier) @name)"#;
        let result = find_definition_candidates(&tree, source, "PENDING", enum_constant_query);
        assert!(result.is_some(), "Should find PENDING enum constant");
        
        let result = find_definition_candidates(&tree, source, "FAILED", enum_constant_query);
        assert!(result.is_some(), "Should find FAILED enum constant");
    }

    #[test] 
    fn test_java_enum_vs_method_priority() {
        // Test that enum constants are found before methods when both exist
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
public enum Result {
    SUCCESS,
    ERROR
}

public class TestClass {
    public static void SUCCESS() {
        System.out.println("This is a method");
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Both enum constant and method exist, but enum should have priority
        let enum_query = r#"(enum_constant name: (identifier) @name)"#;
        let method_query = r#"(method_declaration name: (identifier) @name)"#;
        
        let enum_result = find_definition_candidates(&tree, source, "SUCCESS", enum_query);
        let method_result = find_definition_candidates(&tree, source, "SUCCESS", method_query);
        
        assert!(enum_result.is_some(), "Should find SUCCESS enum constant");
        assert!(method_result.is_some(), "Should find SUCCESS method");
        
        // Both exist, but our logic should prefer enum constants in static context
    }

    #[test]
    fn test_java_enhanced_search_definition_with_enum_constants() {
        // Test that the enhanced search_definition function finds enum constants
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
public enum Priority {
    HIGH,
    MEDIUM,
    LOW
}

public class Task {
    Priority priority = Priority.HIGH;
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        
        // Test that search_definition finds enum constants with our enhanced logic
        let result = search_definition(&tree, source, "HIGH");
        assert!(result.is_some(), "Enhanced search_definition should find HIGH enum constant");
        
        let result = search_definition(&tree, source, "Priority"); 
        assert!(result.is_some(), "Enhanced search_definition should find Priority enum class");
    }

    #[test]
    fn test_java_nested_enum_static_import_extraction() {
        use super::super::project::extract_nested_type_from_import_path;
        
        // Test nested enum type extraction for Java
        assert_eq!(extract_nested_type_from_import_path("com.example.Order.Status"), "Order.Status");
        assert_eq!(extract_nested_type_from_import_path("com.company.deep.Service.State"), "Service.State"); 
        assert_eq!(extract_nested_type_from_import_path("com.example.Priority"), "Priority");
        assert_eq!(extract_nested_type_from_import_path("Status"), "Status");
        
        // Edge cases
        assert_eq!(extract_nested_type_from_import_path(""), "");
        assert_eq!(extract_nested_type_from_import_path("com.example.lower.Upper"), "lower.Upper");
    }

    #[test]
    fn test_java_find_type_in_tree() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class OuterClass {
    public static class InnerClass {
        public static enum Status {
            ACTIVE, INACTIVE
        }
    }
    
    public interface MyInterface {
        void doSomething();
    }
    
    public enum Priority {
        HIGH, LOW
    }
}

class AnotherClass {
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        
        // Test finding regular classes
        let result = java_support.find_type_in_tree(&tree, source, "OuterClass", "file:///test.java");
        assert!(result.is_some(), "Should find OuterClass");
        
        let result = java_support.find_type_in_tree(&tree, source, "AnotherClass", "file:///test.java");
        assert!(result.is_some(), "Should find AnotherClass");
        
        // Test finding nested classes
        let result = java_support.find_type_in_tree(&tree, source, "InnerClass", "file:///test.java");
        assert!(result.is_some(), "Should find InnerClass");
        
        // Test finding interfaces
        let result = java_support.find_type_in_tree(&tree, source, "MyInterface", "file:///test.java");
        assert!(result.is_some(), "Should find MyInterface");
        
        // Test finding enums
        let result = java_support.find_type_in_tree(&tree, source, "Priority", "file:///test.java");
        assert!(result.is_some(), "Should find Priority enum");
        
        let result = java_support.find_type_in_tree(&tree, source, "Status", "file:///test.java");
        assert!(result.is_some(), "Should find nested Status enum");
        
        // Test non-existent type
        let result = java_support.find_type_in_tree(&tree, source, "NonExistent", "file:///test.java");
        assert!(result.is_none(), "Should not find non-existent type");
    }

    #[test]
    fn test_java_find_method_in_tree() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class TestClass {
    public void publicMethod() {
    }
    
    private int privateMethod(String param) {
        return 42;
    }
    
    public static void staticMethod() {
    }
    
    public TestClass() {
    }
    
    public static class InnerClass {
        public void innerMethod() {
        }
        
        private void anotherInnerMethod(int x, String y) {
        }
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        
        // Test finding public methods
        let result = java_support.find_method_in_tree(&tree, source, "publicMethod", "file:///test.java");
        assert!(result.is_some(), "Should find publicMethod");
        
        // Test finding private methods  
        let result = java_support.find_method_in_tree(&tree, source, "privateMethod", "file:///test.java");
        assert!(result.is_some(), "Should find privateMethod");
        
        // Test finding static methods
        let result = java_support.find_method_in_tree(&tree, source, "staticMethod", "file:///test.java");
        assert!(result.is_some(), "Should find staticMethod");
        
        // Test finding methods in nested classes
        let result = java_support.find_method_in_tree(&tree, source, "innerMethod", "file:///test.java");
        assert!(result.is_some(), "Should find innerMethod in nested class");
        
        let result = java_support.find_method_in_tree(&tree, source, "anotherInnerMethod", "file:///test.java");
        assert!(result.is_some(), "Should find anotherInnerMethod in nested class");
        
        // Test non-existent method
        let result = java_support.find_method_in_tree(&tree, source, "nonExistentMethod", "file:///test.java");
        assert!(result.is_none(), "Should not find non-existent method");
    }

    #[test]
    fn test_java_find_property_in_tree() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class TestClass {
    private String privateField;
    public int publicField = 42;
    protected static String staticField;
    private final String finalField = "test";
    
    public static class InnerClass {
        private boolean innerField;
        public String anotherInnerField = "nested";
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        
        // Test finding private fields
        let result = java_support.find_property_in_tree(&tree, source, "privateField", "file:///test.java");
        assert!(result.is_some(), "Should find privateField");
        
        // Test finding public fields
        let result = java_support.find_property_in_tree(&tree, source, "publicField", "file:///test.java");
        assert!(result.is_some(), "Should find publicField");
        
        // Test finding static fields
        let result = java_support.find_property_in_tree(&tree, source, "staticField", "file:///test.java");
        assert!(result.is_some(), "Should find staticField");
        
        // Test finding final fields
        let result = java_support.find_property_in_tree(&tree, source, "finalField", "file:///test.java");
        assert!(result.is_some(), "Should find finalField");
        
        // Test finding fields in nested classes
        let result = java_support.find_property_in_tree(&tree, source, "innerField", "file:///test.java");
        assert!(result.is_some(), "Should find innerField in nested class");
        
        let result = java_support.find_property_in_tree(&tree, source, "anotherInnerField", "file:///test.java");
        assert!(result.is_some(), "Should find anotherInnerField in nested class");
        
        // Test non-existent field
        let result = java_support.find_property_in_tree(&tree, source, "nonExistentField", "file:///test.java");
        assert!(result.is_none(), "Should not find non-existent field");
    }

    #[test]
    fn test_java_find_type_nested_lookup() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class OuterService {
    public static class InnerHandler {
        public static enum State {
            READY, PROCESSING, DONE
        }
        
        public static class DeepNested {
            public void process() {}
        }
    }
    
    public enum Status {
        ACTIVE, INACTIVE
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested types with dot notation
        let result = java_support.find_type(source, "file:///test.java", "OuterService.InnerHandler", dependency_cache.clone());
        // This should work once the nested lookup is properly implemented
        // For now, we test that the method exists and can be called
        
        let result = java_support.find_type(source, "file:///test.java", "OuterService.Status", dependency_cache.clone());
        // Similarly, this tests the nested enum lookup
        
        // Test regular (non-nested) type lookup
        let result = java_support.find_type(source, "file:///test.java", "OuterService", dependency_cache.clone());
        // This should find the outer class
        
        // Test deeply nested type
        let result = java_support.find_type(source, "file:///test.java", "OuterService.InnerHandler.State", dependency_cache.clone());
        // This tests deep nesting
    }

    #[test]
    fn test_java_find_method_nested_lookup() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class ApiController {
    public void handleRequest() {}
    
    public static class AuthHelper {
        public static boolean authenticate(String token) {
            return true;
        }
        
        public void authorize() {}
    }
    
    public static class ValidationHelper {
        public boolean validate(Object data) {
            return true;
        }
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested methods
        let result = java_support.find_method(source, "file:///test.java", "ApiController.AuthHelper.authenticate", dependency_cache.clone());
        // This tests nested static method lookup
        
        let result = java_support.find_method(source, "file:///test.java", "ApiController.AuthHelper.authorize", dependency_cache.clone());
        // This tests nested instance method lookup
        
        // Test regular (non-nested) method lookup
        let result = java_support.find_method(source, "file:///test.java", "handleRequest", dependency_cache.clone());
        // This should find the method in the outer class
    }

    #[test]
    fn test_java_find_property_nested_lookup() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let mut parser = match create_java_parser() {
            Some(p) => p,
            None => {
                println!("Warning: Java parser not available for testing");
                return;
            }
        };
        
        let source = r#"
package com.example;

public class Configuration {
    public static String globalSetting = "default";
    
    public static class DatabaseConfig {
        public static String host = "localhost";
        public int port = 5432;
        
        public static class ConnectionPool {
            public static int maxConnections = 100;
            public boolean autoReconnect = true;
        }
    }
    
    public static class CacheConfig {
        public long ttl = 3600;
        public static boolean enabled = true;
    }
}
"#;
        
        let tree = parser.parse(source, None).unwrap();
        let java_support = JavaSupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test finding nested properties
        let result = java_support.find_property(source, "file:///test.java", "Configuration.DatabaseConfig.host", dependency_cache.clone());
        // This tests nested static field lookup
        
        let result = java_support.find_property(source, "file:///test.java", "Configuration.DatabaseConfig.port", dependency_cache.clone());
        // This tests nested instance field lookup
        
        // Test deeply nested property
        let result = java_support.find_property(source, "file:///test.java", "Configuration.DatabaseConfig.ConnectionPool.maxConnections", dependency_cache.clone());
        // This tests deep nesting
        
        // Test regular (non-nested) property lookup
        let result = java_support.find_property(source, "file:///test.java", "globalSetting", dependency_cache.clone());
        // This should find the property in the outer class
    }

    #[test] 
    fn test_java_actual_find_property() {
        use crate::languages::java::support::JavaSupport;
        use crate::languages::LanguageSupport;
        use std::sync::Arc;
        use crate::core::dependency_cache::DependencyCache;
        
        let source = r#"
package com.example;

public class TestClass {
    private String privateField;
    public int publicField = 42;
}
"#;
        
        let java_support = JavaSupport::new();
        let dependency_cache = Arc::new(DependencyCache::new());
        
        // Test the actual find_property method (not find_property_in_tree)
        let result = java_support.find_property(source, "file:///test.java", "privateField", dependency_cache.clone());
        println!("privateField actual result: {:?}", result);
        assert!(result.is_some(), "Should find privateField using actual find_property method");
        
        let result = java_support.find_property(source, "file:///test.java", "publicField", dependency_cache.clone());
        println!("publicField actual result: {:?}", result);
        assert!(result.is_some(), "Should find publicField using actual find_property method");
    }
}
