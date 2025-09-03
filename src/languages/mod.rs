pub mod common;
pub mod groovy;
pub mod java;
pub mod kotlin;
pub mod traits;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub use traits::LanguageSupport;

/// All supported language implementations for cross-language resolution
pub const ALL_LANGUAGE_SUPPORTS: &[fn() -> Box<dyn LanguageSupport + Send + Sync>] = &[
    || Box::new(crate::languages::java::support::JavaSupport::new()),
    || Box::new(crate::languages::groovy::support::GroovySupport::new()),
    || Box::new(crate::languages::kotlin::support::KotlinSupport::new()),
];

pub struct LanguageRegistry {
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
    extension_map: HashMap<String, String>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self {
            languages: HashMap::new(),
            extension_map: HashMap::new(),
        }
    }

    pub fn register(&mut self, language_id: &str, support: Box<dyn LanguageSupport>) {
        let support: Arc<dyn LanguageSupport> = Arc::from(support);

        // Register language
        self.languages
            .insert(language_id.to_string(), support.clone());

        // Register file extensions
        for ext in support.file_extensions() {
            self.extension_map
                .insert(ext.to_string(), language_id.to_string());
        }
    }

    pub fn detect_language(&self, file_path: &str) -> Option<Arc<dyn LanguageSupport>> {
        let extension = Path::new(file_path).extension()?.to_str()?;

        let ext_with_dot = format!(".{}", extension);
        let language_id = self.extension_map.get(&ext_with_dot)?;

        self.languages.get(language_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLanguageSupport {
        extensions: Vec<String>,
    }

    impl MockLanguageSupport {
        fn new(extensions: Vec<&str>) -> Self {
            Self {
                extensions: extensions.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl LanguageSupport for MockLanguageSupport {
        fn language_id(&self) -> &'static str {
            "mock"
        }

        fn file_extensions(&self) -> &[&'static str] {
            if self.extensions.contains(&".java".to_string()) {
                &[".java"]
            } else {
                &[".mock"]
            }
        }

        fn create_parser(&self) -> tree_sitter::Parser {
            tree_sitter::Parser::new()
        }

        fn collect_diagnostics(&self, _tree: &tree_sitter::Tree, _source: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
            vec![]
        }

        fn find_definition(
            &self,
            _tree: &tree_sitter::Tree,
            _source: &str,
            _position: tower_lsp::lsp_types::Position,
            _file_uri: &str,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
        ) -> anyhow::Result<tower_lsp::lsp_types::Location> {
            Err(anyhow::anyhow!("Not implemented"))
        }

        fn find_implementation(
            &self,
            _tree: &tree_sitter::Tree,
            _source: &str,
            _position: tower_lsp::lsp_types::Position,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
        ) -> anyhow::Result<Vec<tower_lsp::lsp_types::Location>> {
            Ok(vec![])
        }

        fn provide_hover(&self, _tree: &tree_sitter::Tree, _source: &str, _location: tower_lsp::lsp_types::Location) -> Option<tower_lsp::lsp_types::Hover> {
            None
        }

        fn determine_symbol_type_from_context(
            &self,
            _tree: &tree_sitter::Tree,
            _node: &tree_sitter::Node,
            _source: &str,
        ) -> anyhow::Result<crate::core::symbols::SymbolType> {
            Err(anyhow::anyhow!("Not implemented"))
        }

        fn find_definition_chain(
            &self,
            _tree: &tree_sitter::Tree,
            _source: &str,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
            _file_uri: &str,
            _usage_node: &tree_sitter::Node,
        ) -> anyhow::Result<tower_lsp::lsp_types::Location> {
            Err(anyhow::anyhow!("Not implemented"))
        }

        fn find_local(
            &self,
            _tree: &tree_sitter::Tree,
            _source: &str,
            _file_uri: &str,
            _usage_node: &tree_sitter::Node,
        ) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_in_project(
            &self,
            _source: &str,
            _file_uri: &str,
            _usage_node: &tree_sitter::Node,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
        ) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_in_workspace(
            &self,
            _source: &str,
            _file_uri: &str,
            _usage_node: &tree_sitter::Node,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
            _recursion_depth: usize,
        ) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_external(
            &self,
            _source: &str,
            _file_uri: &str,
            _usage_node: &tree_sitter::Node,
            _dependency_cache: std::sync::Arc<crate::core::dependency_cache::DependencyCache>,
        ) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_method_with_signature<'a>(
            &self,
            _tree: &'a tree_sitter::Tree,
            _source: &str,
            _method_name: &str,
            _call_signature: &crate::languages::common::definition_chain::CallSignature,
        ) -> Option<tree_sitter::Node<'a>> {
            None
        }

        fn find_field_declaration_type(&self, _field_name: &str, _tree: &tree_sitter::Tree, _source: &str) -> Option<String> {
            None
        }

        fn find_variable_declaration_type(&self, _variable_name: &str, _tree: &tree_sitter::Tree, _source: &str, _usage_node: &tree_sitter::Node) -> Option<String> {
            None
        }

        fn find_parameter_type(&self, _param_name: &str, _tree: &tree_sitter::Tree, _source: &str, _usage_node: &tree_sitter::Node) -> Option<String> {
            None
        }

        fn set_start_position(
            &self,
            _source: &str,
            _usage_node: &tree_sitter::Node,
            _file_uri: &str,
        ) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn resolve_type_fqn(&self, _type_name: &str, _source: &str, _dependency_cache: &std::sync::Arc<crate::core::dependency_cache::DependencyCache>) -> Option<String> {
            None
        }

        fn find_type_in_tree(&self, _tree: &tree_sitter::Tree, _source: &str, _type_name: &str, _file_uri: &str) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_method_in_tree(&self, _tree: &tree_sitter::Tree, _source: &str, _method_name: &str, _file_uri: &str) -> Option<tower_lsp::lsp_types::Location> {
            None
        }

        fn find_property_in_tree(&self, _tree: &tree_sitter::Tree, _source: &str, _property_name: &str, _file_uri: &str) -> Option<tower_lsp::lsp_types::Location> {
            None
        }
    }

    impl crate::core::queries::QueryProvider for MockLanguageSupport {
        fn method_declaration_queries(&self) -> &[&'static str] {
            &[]
        }

        fn symbol_type_detection_query(&self) -> &'static str {
            "(identifier) @symbol"
        }

        fn import_queries(&self) -> &[&'static str] {
            &[]
        }
    }

    #[test]
    fn test_language_registry_new() {
        let registry = LanguageRegistry::new();
        assert_eq!(registry.languages.len(), 0);
        assert_eq!(registry.extension_map.len(), 0);
    }

    #[test]
    fn test_language_registry_register() {
        let mut registry = LanguageRegistry::new();
        let mock_support = MockLanguageSupport::new(vec![".mock", ".test"]);
        
        registry.register("mock_lang", Box::new(mock_support));
        
        assert_eq!(registry.languages.len(), 1);
        assert_eq!(registry.extension_map.len(), 1); // Only uses file_extensions() method
        assert!(registry.languages.contains_key("mock_lang"));
        assert!(registry.extension_map.contains_key(".mock"));
    }

    #[test]
    fn test_language_registry_detect_language_existing() {
        let mut registry = LanguageRegistry::new();
        let mock_support = MockLanguageSupport::new(vec![".mock"]);
        
        registry.register("mock_lang", Box::new(mock_support));
        
        let result = registry.detect_language("test_file.mock");
        assert!(result.is_some());
        
        if let Some(language) = result {
            assert_eq!(language.language_id(), "mock");
        }
    }

    #[test]
    fn test_language_registry_detect_language_non_existing() {
        let registry = LanguageRegistry::new();
        
        let result = registry.detect_language("test_file.unknown");
        assert!(result.is_none());
    }

    #[test]
    fn test_language_registry_detect_language_no_extension() {
        let mut registry = LanguageRegistry::new();
        let mock_support = MockLanguageSupport::new(vec![".mock"]);
        
        registry.register("mock_lang", Box::new(mock_support));
        
        let result = registry.detect_language("file_without_extension");
        assert!(result.is_none());
    }

    #[test]
    fn test_language_registry_detect_language_complex_path() {
        let mut registry = LanguageRegistry::new();
        let mock_support = MockLanguageSupport::new(vec![".java"]);
        
        registry.register("java", Box::new(mock_support));
        
        let result = registry.detect_language("/path/to/complex/TestClass.java");
        assert!(result.is_some());
    }

    #[test]
    fn test_all_language_supports_initialization() {
        // Test that ALL_LANGUAGE_SUPPORTS can be called without panicking
        for language_fn in ALL_LANGUAGE_SUPPORTS {
            let _language = language_fn();
            // Just verify it creates successfully
        }
        
        // Verify we have the expected number of language supports
        assert_eq!(ALL_LANGUAGE_SUPPORTS.len(), 3); // Java, Groovy, Kotlin
    }
}
