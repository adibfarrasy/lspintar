pub mod groovy;
pub mod java;
pub mod kotlin;
pub mod traits;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub use traits::LanguageSupport;

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

    pub fn get_language(&self, language_id: &str) -> Option<Arc<dyn LanguageSupport>> {
        self.languages.get(language_id).cloned()
    }

    pub fn supported_extensions(&self) -> Vec<&str> {
        self.extension_map.keys().map(|s| s.as_str()).collect()
    }
}
