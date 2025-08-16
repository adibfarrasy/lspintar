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

#[cfg(test)]
mod tests {
    use super::*;

    struct SymbolTypeTestCase {
        name: &'static str,
        symbol_type: SymbolType,
        expected_is_declaration: bool,
    }

    #[test]
    fn test_symbol_type_is_declaration() {
        let test_cases = vec![
            // Declaration types
            SymbolTypeTestCase {
                name: "variable declaration",
                symbol_type: SymbolType::VariableDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "function declaration",
                symbol_type: SymbolType::FunctionDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "method declaration",
                symbol_type: SymbolType::MethodDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "class declaration",
                symbol_type: SymbolType::ClassDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "interface declaration",
                symbol_type: SymbolType::InterfaceDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "annotation declaration",
                symbol_type: SymbolType::AnnotationDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "enum declaration",
                symbol_type: SymbolType::EnumDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "field declaration",
                symbol_type: SymbolType::FieldDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "property declaration",
                symbol_type: SymbolType::PropertyDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "module declaration",
                symbol_type: SymbolType::ModuleDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "package declaration",
                symbol_type: SymbolType::PackageDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "constant declaration",
                symbol_type: SymbolType::ConstantDeclaration,
                expected_is_declaration: true,
            },
            SymbolTypeTestCase {
                name: "parameter declaration",
                symbol_type: SymbolType::ParameterDeclaration,
                expected_is_declaration: true,
            },
            
            // Usage types (not declarations)
            SymbolTypeTestCase {
                name: "variable usage",
                symbol_type: SymbolType::VariableUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "function call",
                symbol_type: SymbolType::FunctionCall,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "method call",
                symbol_type: SymbolType::MethodCall,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "super class",
                symbol_type: SymbolType::SuperClass,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "super interface",
                symbol_type: SymbolType::SuperInterface,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "annotation usage",
                symbol_type: SymbolType::AnnotationUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "enum usage",
                symbol_type: SymbolType::EnumUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "field usage",
                symbol_type: SymbolType::FieldUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "property usage",
                symbol_type: SymbolType::PropertyUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "module usage",
                symbol_type: SymbolType::ModuleUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "package usage",
                symbol_type: SymbolType::PackageUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "constant usage",
                symbol_type: SymbolType::ConstantUsage,
                expected_is_declaration: false,
            },
            SymbolTypeTestCase {
                name: "type",
                symbol_type: SymbolType::Type,
                expected_is_declaration: false,
            },
        ];

        for test_case in test_cases {
            let result = test_case.symbol_type.is_declaration();
            assert_eq!(
                result,
                test_case.expected_is_declaration,
                "Test '{}': expected is_declaration() = {}, got {}",
                test_case.name,
                test_case.expected_is_declaration,
                result
            );
        }
    }

    #[test]
    fn test_symbol_type_clone_and_debug() {
        let symbol = SymbolType::ClassDeclaration;
        let cloned = symbol.clone();
        
        assert_eq!(symbol, cloned);
        assert!(format!("{:?}", symbol).contains("ClassDeclaration"));
    }

    #[test]
    fn test_symbol_type_equality() {
        let test_cases = vec![
            (SymbolType::VariableDeclaration, SymbolType::VariableDeclaration, true),
            (SymbolType::VariableDeclaration, SymbolType::VariableUsage, false),
            (SymbolType::MethodCall, SymbolType::MethodCall, true),
            (SymbolType::ClassDeclaration, SymbolType::InterfaceDeclaration, false),
        ];

        for (symbol1, symbol2, expected_equal) in test_cases {
            let result = symbol1 == symbol2;
            assert_eq!(
                result,
                expected_equal,
                "Equality test failed: {:?} == {:?} should be {}",
                symbol1,
                symbol2,
                expected_equal
            );
        }
    }
}