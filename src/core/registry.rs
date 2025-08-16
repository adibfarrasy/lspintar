use tower_lsp::lsp_types::Location;
use std::sync::Arc;
use std::collections::HashMap;

use crate::core::dependency_cache::DependencyCache;
use crate::languages::traits::LanguageSupport;

/// Registry for managing multiple language supports
pub struct LanguageRegistry {
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self {
            languages: HashMap::new(),
        }
    }
    
    /// Register a language support implementation
    pub fn register(&mut self, language_support: Arc<dyn LanguageSupport>) {
        let language_id = language_support.language_id().to_string();
        self.languages.insert(language_id, language_support);
    }
    
    /// Get a language support by ID
    pub fn get_language(&self, language_id: &str) -> Option<Arc<dyn LanguageSupport>> {
        self.languages.get(language_id).cloned()
    }
    
    /// Get language support by file extension
    pub fn get_language_for_file(&self, file_path: &str) -> Option<Arc<dyn LanguageSupport>> {
        for language in self.languages.values() {
            for extension in language.file_extensions() {
                if file_path.ends_with(extension) {
                    return Some(language.clone());
                }
            }
        }
        None
    }
    
    /// Get all supported language IDs
    pub fn get_supported_languages(&self) -> Vec<&str> {
        self.languages.keys().map(|k| k.as_str()).collect()
    }
    
    /// Resolve a symbol across all registered languages
    pub fn resolve_symbol_cross_language(&self, symbol: &str, from_language: &str) -> Option<Location> {
        // TODO: Implement cross-language symbol resolution
        // This should:
        // 1. Try the source language first
        // 2. Try compatible target languages based on interop rules
        // 3. Use dependency cache to search efficiently
        
        None
    }
    
    /// Resolve a cross-language symbol with dependency cache
    pub fn resolve_cross_language_symbol(&self, symbol: &str, target_language: &str, dependency_cache: Arc<DependencyCache>) -> Option<Location> {
        // TODO: Implement targeted cross-language resolution
        // This should use the dependency cache for efficient searching
        
        None
    }
    
    /// Check if two languages can interoperate
    pub fn can_interoperate(&self, from_language: &str, to_language: &str) -> bool {
        // JVM language interop rules
        let jvm_languages = ["java", "groovy", "kotlin"];
        
        // All JVM languages can generally interoperate
        jvm_languages.contains(&from_language) && jvm_languages.contains(&to_language)
    }
    
    /// Get all file extensions supported by the registry
    pub fn get_all_extensions(&self) -> Vec<&str> {
        let mut extensions = Vec::new();
        for language in self.languages.values() {
            extensions.extend(language.file_extensions());
        }
        extensions.sort();
        extensions.dedup();
        extensions
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}