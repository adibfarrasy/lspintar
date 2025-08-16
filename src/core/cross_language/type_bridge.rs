/// Type information that can be shared across languages
#[derive(Debug, Clone)]
pub struct CrossLanguageTypeInfo {
    pub name: String,
    pub full_qualified_name: String,
    pub language: String,
    pub kind: TypeKind,
    pub visibility: Visibility,
    pub methods: Vec<MethodInfo>,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Class,
    Interface,
    Enum,
    Annotation,
    Object, // Kotlin object
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
    Package, // Default/package-private
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: String,
    pub parameters: Vec<ParameterInfo>,
    pub return_type: String,
    pub visibility: Visibility,
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub type_name: String,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_final: bool,
}

/// Bridge for converting type information between languages
pub struct TypeBridge;

impl TypeBridge {
    /// Convert Java type information to a format usable by Groovy
    pub fn java_to_groovy_type(java_type: &CrossLanguageTypeInfo) -> Option<CrossLanguageTypeInfo> {
        // TODO: Implement Java -> Groovy type conversion
        // This should handle:
        // - Java classes that are accessible from Groovy
        // - Type name conversions (e.g., primitive types)
        // - Method signature adaptations
        
        None
    }
    
    /// Convert Kotlin type information to a format usable by Java
    pub fn kotlin_to_java_type(kotlin_type: &CrossLanguageTypeInfo) -> Option<CrossLanguageTypeInfo> {
        // TODO: Implement Kotlin -> Java type conversion
        // This should handle:
        // - Kotlin classes compiled to JVM bytecode
        // - Nullable type annotations
        // - Extension functions visibility
        
        None
    }
    
    /// Convert Groovy type information to a format usable by Java
    pub fn groovy_to_java_type(groovy_type: &CrossLanguageTypeInfo) -> Option<CrossLanguageTypeInfo> {
        // TODO: Implement Groovy -> Java type conversion
        // This should handle:
        // - Groovy classes compiled to JVM bytecode
        // - Dynamic typing vs static typing
        // - Groovy-specific features
        
        None
    }
    
    /// Check if a type from one language is compatible with another language
    pub fn is_compatible(type_info: &CrossLanguageTypeInfo, target_language: &str) -> bool {
        // Basic JVM interop rules
        match (type_info.language.as_str(), target_language) {
            // Public classes are generally accessible across JVM languages
            ("java", "groovy") | ("java", "kotlin") => type_info.visibility == Visibility::Public,
            ("kotlin", "java") | ("kotlin", "groovy") => type_info.visibility == Visibility::Public,
            ("groovy", "java") | ("groovy", "kotlin") => type_info.visibility == Visibility::Public,
            
            // Same language is always compatible
            (a, b) if a == b => true,
            
            _ => false,
        }
    }
}