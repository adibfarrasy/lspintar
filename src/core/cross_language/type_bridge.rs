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

