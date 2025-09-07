//! Groovy-specific type system extensions
//!
//! This module provides Groovy/Java-specific type hints and compatibility rules
//! that extend the core type system.

use crate::core::types::{TypeHint, Confidence};

/// Groovy-specific type hint extensions
impl TypeHint {
    /// Create unknown type for Groovy (defaults to java.lang.Object)
    pub fn groovy_unknown() -> Self {
        Self {
            display_name: "Object".to_string(),
            qualified_name: Some("java.lang.Object".to_string()),
            confidence: Confidence::Low,
        }
    }
    
    /// Groovy String type
    pub fn groovy_string() -> Self {
        Self::known("String", "java.lang.String")
    }
    
    /// Groovy Integer type  
    pub fn groovy_integer() -> Self {
        Self::known("Integer", "java.lang.Integer")
    }
    
    /// Groovy Boolean type
    pub fn groovy_boolean() -> Self {
        Self::known("Boolean", "java.lang.Boolean")
    }
    
    /// Groovy List type with element type
    pub fn groovy_list(element_hint: &str) -> Self {
        Self {
            display_name: format!("List<{}>", element_hint),
            qualified_name: Some("java.util.List".to_string()),
            confidence: Confidence::Medium,
        }
    }
    
    /// Groovy Map type with key and value types
    pub fn groovy_map(key_hint: &str, value_hint: &str) -> Self {
        Self {
            display_name: format!("Map<{}, {}>", key_hint, value_hint),
            qualified_name: Some("java.util.Map".to_string()),
            confidence: Confidence::Medium,
        }
    }

    /// Groovy Long type
    pub fn groovy_long() -> Self {
        Self::known("Long", "java.lang.Long")
    }

    /// Groovy Double type
    pub fn groovy_double() -> Self {
        Self::known("Double", "java.lang.Double")
    }

    /// Groovy Float type
    pub fn groovy_float() -> Self {
        Self::known("Float", "java.lang.Float")
    }

    /// Groovy BigDecimal type (default for decimal literals)
    pub fn groovy_bigdecimal() -> Self {
        Self::known("BigDecimal", "java.math.BigDecimal")
    }
}

/// Groovy-specific compatibility rules
/// This extends the basic compatibility check with Java/Groovy-specific logic
pub fn groovy_types_compatible(hint1: &TypeHint, hint2: &TypeHint) -> bool {
    // Always assume compatibility if either is low confidence
    if hint1.confidence == Confidence::Low || hint2.confidence == Confidence::Low {
        return true;
    }
    
    // Groovy/Java-specific: Object is compatible with everything
    if let (Some(q1), Some(q2)) = (&hint1.qualified_name, &hint2.qualified_name) {
        return q1 == q2 || q1 == "java.lang.Object" || q2 == "java.lang.Object";
    }
    
    // Otherwise, just compare display names
    hint1.display_name == hint2.display_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_groovy_basic_types() {
        let string = TypeHint::groovy_string();
        assert_eq!(string.display_name, "String");
        assert_eq!(string.qualified_name, Some("java.lang.String".to_string()));
        assert_eq!(string.confidence, Confidence::High);

        let integer = TypeHint::groovy_integer();
        assert_eq!(integer.display_name, "Integer");
        assert_eq!(integer.qualified_name, Some("java.lang.Integer".to_string()));

        let boolean = TypeHint::groovy_boolean();
        assert_eq!(boolean.display_name, "Boolean");
        assert_eq!(boolean.qualified_name, Some("java.lang.Boolean".to_string()));

        let unknown = TypeHint::groovy_unknown();
        assert_eq!(unknown.display_name, "Object");
        assert_eq!(unknown.qualified_name, Some("java.lang.Object".to_string()));
        assert_eq!(unknown.confidence, Confidence::Low);
    }

    #[test]
    fn test_groovy_numeric_types() {
        let long = TypeHint::groovy_long();
        assert_eq!(long.display_name, "Long");
        assert_eq!(long.qualified_name, Some("java.lang.Long".to_string()));

        let double = TypeHint::groovy_double();
        assert_eq!(double.display_name, "Double");
        assert_eq!(double.qualified_name, Some("java.lang.Double".to_string()));

        let float = TypeHint::groovy_float();
        assert_eq!(float.display_name, "Float");
        assert_eq!(float.qualified_name, Some("java.lang.Float".to_string()));

        let bigdecimal = TypeHint::groovy_bigdecimal();
        assert_eq!(bigdecimal.display_name, "BigDecimal");
        assert_eq!(bigdecimal.qualified_name, Some("java.math.BigDecimal".to_string()));
    }

    #[test]
    fn test_groovy_collection_types() {
        let list = TypeHint::groovy_list("String");
        assert_eq!(list.display_name, "List<String>");
        assert_eq!(list.qualified_name, Some("java.util.List".to_string()));
        assert_eq!(list.confidence, Confidence::Medium);

        let map = TypeHint::groovy_map("String", "Integer");
        assert_eq!(map.display_name, "Map<String, Integer>");
        assert_eq!(map.qualified_name, Some("java.util.Map".to_string()));
        assert_eq!(map.confidence, Confidence::Medium);
    }

    #[test]
    fn test_groovy_compatibility() {
        let string = TypeHint::groovy_string();
        let integer = TypeHint::groovy_integer();
        let object = TypeHint::groovy_unknown();
        
        // Same types should be compatible
        assert!(groovy_types_compatible(&string, &string));
        
        // Different types should not be compatible
        assert!(!groovy_types_compatible(&string, &integer));
        
        // Object should be compatible with everything
        assert!(groovy_types_compatible(&string, &object));
        assert!(groovy_types_compatible(&object, &integer));
        assert!(groovy_types_compatible(&object, &object));
    }
}