//! Type system re-exports and backwards compatibility
//!
//! This module maintains backwards compatibility by re-exporting the core type system
//! and providing legacy convenience methods that delegate to Groovy-specific implementations.

use tower_lsp::lsp_types::Position as LspPosition;

pub type Position = LspPosition;

// Re-export core types for backwards compatibility
pub use crate::core::types::{TypeHint, Confidence};

/// Backwards compatibility layer for existing TypeHint usage
impl TypeHint {
    /// Legacy method - delegates to groovy_unknown()
    #[deprecated(note = "Use TypeHint::groovy_unknown() for explicit language support")]
    pub fn unknown() -> Self {
        Self::groovy_unknown()
    }

    /// Legacy method - delegates to groovy_string()
    #[deprecated(note = "Use TypeHint::groovy_string() for explicit language support")]
    pub fn string() -> Self {
        Self::groovy_string()
    }

    /// Legacy method - delegates to groovy_integer()
    #[deprecated(note = "Use TypeHint::groovy_integer() for explicit language support")]
    pub fn integer() -> Self {
        Self::groovy_integer()
    }

    /// Legacy method - delegates to groovy_boolean()
    #[deprecated(note = "Use TypeHint::groovy_boolean() for explicit language support")]
    pub fn boolean() -> Self {
        Self::groovy_boolean()
    }

    /// Legacy method - delegates to groovy_list()
    #[deprecated(note = "Use TypeHint::groovy_list() for explicit language support")]
    pub fn list(element_hint: &str) -> Self {
        Self::groovy_list(element_hint)
    }

    /// Legacy method - delegates to groovy_map()
    #[deprecated(note = "Use TypeHint::groovy_map() for explicit language support")]
    pub fn map(key_hint: &str, value_hint: &str) -> Self {
        Self::groovy_map(key_hint, value_hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backwards_compatibility() {
        // Test that legacy methods still work
        #[allow(deprecated)]
        {
            let string_hint = TypeHint::string();
            assert_eq!(string_hint.display_name, "String");
            assert_eq!(string_hint.qualified_name, Some("java.lang.String".to_string()));
            assert_eq!(string_hint.confidence, Confidence::High);

            let list_hint = TypeHint::list("String");
            assert_eq!(list_hint.display_name, "List<String>");
            assert_eq!(list_hint.qualified_name, Some("java.util.List".to_string()));
            assert_eq!(list_hint.confidence, Confidence::Medium);

            let map_hint = TypeHint::map("String", "Integer");
            assert_eq!(map_hint.display_name, "Map<String, Integer>");

            let unknown = TypeHint::unknown();
            assert_eq!(unknown.display_name, "Object");
            assert_eq!(unknown.qualified_name, Some("java.lang.Object".to_string()));
            assert_eq!(unknown.confidence, Confidence::Low);
        }
    }

    #[test]
    fn test_new_methods_work() {
        // Test that new methods work
        let string_hint = TypeHint::groovy_string();
        assert_eq!(string_hint.display_name, "String");
        
        let list_hint = TypeHint::groovy_list("Integer");
        assert_eq!(list_hint.display_name, "List<Integer>");
        
        let unknown = TypeHint::groovy_unknown();
        assert_eq!(unknown.display_name, "Object");
    }

    #[test]
    fn test_compatibility_still_works() {
        #[allow(deprecated)]
        {
            let string1 = TypeHint::string();
            let string2 = TypeHint::string();
            let integer = TypeHint::integer();
            let unknown = TypeHint::unknown();
            
            assert!(string1.might_be_compatible_with(&string2));
            assert!(!string1.might_be_compatible_with(&integer));
            assert!(string1.might_be_compatible_with(&unknown));
            assert!(unknown.might_be_compatible_with(&integer));
        }
    }
}