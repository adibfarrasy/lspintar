//! Core type system for LSPintar - language-agnostic type hints
//!
//! This provides the foundation for type inference across all supported languages.
//! Language-specific extensions should be implemented in their respective modules.

/// Simple type hint for IDE features like hover and completion
#[derive(Debug, Clone, PartialEq)]
pub struct TypeHint {
    /// Display name for the user (e.g., "String", "List<String>", "MyClass")
    pub display_name: String,
    /// Fully qualified name if known (e.g., "java.lang.String", "kotlin.String")
    pub qualified_name: Option<String>,
    /// Confidence level of this inference
    pub confidence: Confidence,
}

/// How confident we are about a type inference
#[derive(Debug, Clone, PartialEq)]
pub enum Confidence {
    /// We're very sure (e.g., string literal -> String)
    High,
    /// Reasonable guess (e.g., method call with known return type)
    Medium,
    /// Wild guess or fallback
    Low,
}

impl TypeHint {
    /// Create a high-confidence type hint with known qualified name
    pub fn known(display_name: &str, qualified_name: &str) -> Self {
        Self {
            display_name: display_name.to_string(),
            qualified_name: Some(qualified_name.to_string()),
            confidence: Confidence::High,
        }
    }

    /// Create a medium-confidence type hint without qualified name
    pub fn likely(display_name: &str) -> Self {
        Self {
            display_name: display_name.to_string(),
            qualified_name: None,
            confidence: Confidence::Medium,
        }
    }

    /// Language-agnostic compatibility check
    /// This is a basic check - languages should implement their own compatibility rules
    pub fn might_be_compatible_with(&self, other: &TypeHint) -> bool {
        // Always assume compatibility if either is low confidence
        if self.confidence == Confidence::Low || other.confidence == Confidence::Low {
            return true;
        }
        
        // If we have qualified names, use those
        if let (Some(q1), Some(q2)) = (&self.qualified_name, &other.qualified_name) {
            return q1 == q2;
        }
        
        // Otherwise, just compare display names
        self.display_name == other.display_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_hint_creation() {
        let hint = TypeHint::known("String", "java.lang.String");
        assert_eq!(hint.display_name, "String");
        assert_eq!(hint.qualified_name, Some("java.lang.String".to_string()));
        assert_eq!(hint.confidence, Confidence::High);
    }

    #[test]
    fn test_likely_type_hint() {
        let hint = TypeHint::likely("SomeClass");
        assert_eq!(hint.display_name, "SomeClass");
        assert_eq!(hint.qualified_name, None);
        assert_eq!(hint.confidence, Confidence::Medium);
    }

    #[test]
    fn test_basic_compatibility() {
        let string1 = TypeHint::known("String", "java.lang.String");
        let string2 = TypeHint::known("String", "java.lang.String");
        let integer = TypeHint::known("Integer", "java.lang.Integer");
        let low_conf = TypeHint {
            display_name: "Unknown".to_string(),
            qualified_name: None,
            confidence: Confidence::Low,
        };
        
        assert!(string1.might_be_compatible_with(&string2));
        assert!(!string1.might_be_compatible_with(&integer));
        assert!(string1.might_be_compatible_with(&low_conf)); // Low confidence is always compatible
        assert!(low_conf.might_be_compatible_with(&integer)); // Low confidence is always compatible
    }

    #[test]
    fn test_confidence_levels() {
        let high = TypeHint::known("MyClass", "com.example.MyClass");
        let medium = TypeHint::likely("SomeClass");
        let low = TypeHint {
            display_name: "Unknown".to_string(),
            qualified_name: None,
            confidence: Confidence::Low,
        };
        
        assert_eq!(high.confidence, Confidence::High);
        assert_eq!(medium.confidence, Confidence::Medium);
        assert_eq!(low.confidence, Confidence::Low);
    }
}