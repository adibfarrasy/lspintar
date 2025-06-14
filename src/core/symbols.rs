#[derive(Debug, Clone, PartialEq)]
pub enum SymbolType {
    VariableDeclaration,
    VariableUsage,
    FunctionDeclaration,
    FunctionCall,
    MethodDeclaration,
    MethodCall,
    ClassDeclaration,
    SuperClass,
    InterfaceDeclaration,
    SuperInterface,
    AnnotationDeclaration,
    AnnotationUsage,
    EnumDeclaration,
    EnumUsage,
    FieldDeclaration,
    FieldUsage,
    PropertyDeclaration,
    PropertyUsage,
    ModuleDeclaration,
    ModuleUsage,
    PackageDeclaration,
    PackageUsage,
    ConstantDeclaration,
    ConstantUsage,
    ParameterDeclaration,
    Type,
}

impl SymbolType {
    pub fn is_declaration(&self) -> bool {
        matches!(
            self,
            Self::VariableDeclaration
                | Self::FunctionDeclaration
                | Self::MethodDeclaration
                | Self::ClassDeclaration
                | Self::InterfaceDeclaration
                | Self::AnnotationDeclaration
                | Self::EnumDeclaration
                | Self::FieldDeclaration
                | Self::PropertyDeclaration
                | Self::ModuleDeclaration
                | Self::PackageDeclaration
                | Self::ConstantDeclaration
                | Self::ParameterDeclaration
        )
    }
}
