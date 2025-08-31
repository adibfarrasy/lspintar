use std::sync::Arc;

use anyhow::Result;
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Node, Parser, Tree};

use crate::core::{dependency_cache::DependencyCache, queries::QueryProvider, symbols::SymbolType};

pub trait LanguageSupport: Send + Sync + QueryProvider {
    fn language_id(&self) -> &'static str;

    fn file_extensions(&self) -> &[&'static str];

    fn create_parser(&self) -> Parser;

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic>;

    fn find_definition(
        &self,
        _tree: &Tree,
        _source: &str,
        position: Position,
        uri: &str,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location>;

    fn find_implementation(
        &self,
        _tree: &Tree,
        _source: &str,
        position: Position,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>>;

    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover>;

    fn determine_symbol_type_from_context(
        &self,
        _tree: &Tree,
        node: &Node,
        _source: &str,
    ) -> Result<SymbolType>;

    fn find_definition_chain(
        &self,
        _tree: &Tree,
        _source: &str,
        _dependency_cache: Arc<DependencyCache>,
        _file_uri: &str,
        _usage_node: &Node,
    ) -> Result<Location>;

    fn find_local(
        &self,
        _tree: &Tree,
        _source: &str,
        _file_uri: &str,
        _usage_node: &Node,
    ) -> Option<Location>;

    fn find_in_project(
        &self,
        _source: &str,
        _file_uri: &str,
        _usage_node: &Node,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location>;

    fn find_in_workspace(
        &self,
        _source: &str,
        _file_uri: &str,
        _usage_node: &Node,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location>;

    fn find_external(
        &self,
        _source: &str,
        _file_uri: &str,
        _usage_node: &Node,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location>;

    /// Find a method with signature matching for overload resolution
    fn find_method_with_signature<'a>(
        &self,
        tree: &'a Tree,
        _source: &str,
        _method_name: &str,
        call_signature: &crate::languages::common::method_resolution::CallSignature,
    ) -> Option<tree_sitter::Node<'a>>;

    /// Find field/property declaration and return its type
    /// Each language implements this according to its AST structure
    fn find_field_declaration_type(&self, field_name: &str, tree: &Tree, source: &str) -> Option<String>;
    
    /// Find variable declaration and return its type  
    /// Each language implements this according to its AST structure
    fn find_variable_declaration_type(&self, variable_name: &str, tree: &Tree, source: &str, usage_node: &Node) -> Option<String>;
    
    /// Find parameter declaration and return its type
    /// Each language implements this according to its AST structure  
    fn find_parameter_type(&self, param_name: &str, tree: &Tree, source: &str, usage_node: &Node) -> Option<String>;

    fn set_start_position(
        &self,
        _source: &str,
        _usage_node: &Node,
        _file_uri: &str,
    ) -> Option<Location>;

    /// Extract static method context (ClassName.methodName) from usage node
    /// Returns (class_name, method_name) if this is a static method call
    fn extract_static_method_context(
        &self,
        usage_node: &Node,
        source: &str,
    ) -> Option<(String, String)> {
        // Default implementation using common JVM patterns
        crate::languages::common::method_resolution::extract_static_method_context(usage_node, source)
    }

    /// Extract instance method context (variable.methodName) from usage node  
    /// Returns (variable_name, method_name) if this is an instance method call
    fn extract_instance_method_context(
        &self,
        usage_node: &Node,
        source: &str,
    ) -> Option<(String, String)> {
        // Default implementation using common JVM patterns
        crate::languages::common::method_resolution::extract_instance_method_context(usage_node, source)
    }


    /// Resolve instance method definition by finding variable type and then method within it
    fn find_instance_method_definition(
        &self,
        _tree: &Tree,
        _source: &str,
        _file_uri: &str,
        _usage_node: &Node,
        _variable_name: &str,
        _method_name: &str,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // Default implementation - languages can override for specific behavior
        None
    }

    /// Extract call signature from the usage context for method overload resolution
    /// Each language can provide its own implementation for better signature matching
    fn extract_call_signature(&self, usage_node: &Node, source: &str) -> Option<crate::languages::common::method_resolution::CallSignature> {
        // Default implementation uses Groovy's signature extraction (for backward compatibility)
        crate::languages::common::method_resolution::extract_call_signature_from_context(usage_node, source)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::symbols::SymbolType;
    use anyhow::Result;
    use std::sync::Arc;
    use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position, Range, Url};
    use tree_sitter::{Node, Parser, Tree};

    // Mock implementation for testing
    struct MockLanguageSupport {
        language_id: &'static str,
        extensions: &'static [&'static str],
        should_find_local: bool,
        should_find_project: bool,
        should_find_workspace: bool,
        should_find_external: bool,
        symbol_type: SymbolType,
    }

    impl MockLanguageSupport {
        fn new(language_id: &'static str) -> Self {
            Self {
                language_id,
                extensions: &[".mock"],
                should_find_local: false,
                should_find_project: false,
                should_find_workspace: false,
                should_find_external: false,
                symbol_type: SymbolType::Type,
            }
        }

        fn with_local_resolution(mut self) -> Self {
            self.should_find_local = true;
            self
        }

        fn with_project_resolution(mut self) -> Self {
            self.should_find_project = true;
            self
        }

        fn with_workspace_resolution(mut self) -> Self {
            self.should_find_workspace = true;
            self
        }

        fn with_external_resolution(mut self) -> Self {
            self.should_find_external = true;
            self
        }

        fn with_symbol_type(mut self, symbol_type: SymbolType) -> Self {
            self.symbol_type = symbol_type;
            self
        }
    }

    impl QueryProvider for MockLanguageSupport {
        fn method_declaration_queries(&self) -> &[&'static str] {
            &[]
        }

        fn symbol_type_detection_query(&self) -> &'static str {
            ""
        }

        fn import_queries(&self) -> &[&'static str] {
            &[]
        }
    }

    impl LanguageSupport for MockLanguageSupport {
        fn language_id(&self) -> &'static str {
            self.language_id
        }

        fn file_extensions(&self) -> &[&'static str] {
            self.extensions
        }

        fn create_parser(&self) -> Parser {
            Parser::new()
        }

        fn collect_diagnostics(&self, _tree: &Tree, _source: &str) -> Vec<Diagnostic> {
            vec![]
        }

        fn find_definition(
            &self,
            tree: &Tree,
            source: &str,
            _position: Position,
            uri: &str,
            dependency_cache: Arc<DependencyCache>,
        ) -> Result<Location> {
            // Use the mock node as _usage_node
            let root_node = tree.root_node();
            self.find_definition_chain(tree, source, dependency_cache, uri, &root_node)
        }

        fn find_implementation(
            &self,
            _tree: &Tree,
            _source: &str,
            _position: Position,
            _dependency_cache: Arc<DependencyCache>,
        ) -> Result<Vec<Location>> {
            Ok(vec![])
        }

        fn provide_hover(&self, _tree: &Tree, _source: &str, _location: Location) -> Option<Hover> {
            None
        }

        fn determine_symbol_type_from_context(
            &self,
            _tree: &Tree,
            _node: &Node,
            _source: &str,
        ) -> Result<SymbolType> {
            Ok(self.symbol_type.clone())
        }

        fn find_method_with_signature<'a>(
            &self,
            _tree: &'a Tree,
            _source: &str,
            _method_name: &str,
            _call_signature: &crate::languages::common::method_resolution::CallSignature,
        ) -> Option<tree_sitter::Node<'a>> {
            None
        }

        fn find_field_declaration_type(&self, _field_name: &str, _tree: &Tree, _source: &str) -> Option<String> {
            None
        }

        fn find_variable_declaration_type(&self, _variable_name: &str, _tree: &Tree, _source: &str, _usage_node: &Node) -> Option<String> {
            None
        }

        fn find_parameter_type(&self, _param_name: &str, _tree: &Tree, _source: &str, _usage_node: &Node) -> Option<String> {
            None
        }

        fn find_definition_chain(
            &self,
            tree: &Tree,
            source: &str,
            dependency_cache: Arc<DependencyCache>,
            uri: &str,
            usage_node: &Node,
        ) -> Result<Location> {
            self.find_local(tree, source, uri, usage_node)
                .or_else(|| self.find_in_project(source, uri, usage_node, dependency_cache.clone()))
                .or_else(|| {
                    self.find_in_workspace(source, uri, usage_node, dependency_cache.clone())
                })
                .or_else(|| self.find_external(source, uri, usage_node, dependency_cache.clone()))
                .and_then(|location| {
                    self.set_start_position(source, usage_node, &location.uri.to_string())
                })
                .ok_or_else(|| anyhow::anyhow!("Definition not found"))
        }

        fn find_local(
            &self,
            _tree: &Tree,
            _source: &str,
            file_uri: &str,
            _usage_node: &Node,
        ) -> Option<Location> {
            if self.should_find_local {
                Some(self.create_mock_location(file_uri))
            } else {
                None
            }
        }

        fn find_in_project(
            &self,
            _source: &str,
            file_uri: &str,
            _usage_node: &Node,
            _dependency_cache: Arc<DependencyCache>,
        ) -> Option<Location> {
            if self.should_find_project {
                Some(self.create_mock_location(file_uri))
            } else {
                None
            }
        }

        fn find_in_workspace(
            &self,
            _source: &str,
            file_uri: &str,
            _usage_node: &Node,
            _dependency_cache: Arc<DependencyCache>,
        ) -> Option<Location> {
            if self.should_find_workspace {
                Some(self.create_mock_location(file_uri))
            } else {
                None
            }
        }

        fn find_external(
            &self,
            _source: &str,
            file_uri: &str,
            _usage_node: &Node,
            _dependency_cache: Arc<DependencyCache>,
        ) -> Option<Location> {
            if self.should_find_external {
                Some(self.create_mock_location(file_uri))
            } else {
                None
            }
        }

        fn set_start_position(
            &self,
            _source: &str,
            _usage_node: &Node,
            file_uri: &str,
        ) -> Option<Location> {
            Some(self.create_mock_location(file_uri))
        }
    }

    impl MockLanguageSupport {
        fn create_mock_location(&self, file_uri: &str) -> Location {
            Location {
                uri: Url::parse(file_uri).unwrap(),
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 10,
                    },
                },
            }
        }
    }

    struct LanguageSupportTestCase {
        name: &'static str,
        language_support: MockLanguageSupport,
        expected_language_id: &'static str,
        expected_extensions: &'static [&'static str],
    }

    struct DefinitionChainTestCase {
        name: &'static str,
        language_support: MockLanguageSupport,
        expected_resolution_level: &'static str, // "local", "project", "workspace", "external", "none"
    }

    #[test]
    fn test_language_support_basic_properties() {
        let test_cases = vec![
            LanguageSupportTestCase {
                name: "groovy language support",
                language_support: MockLanguageSupport::new("groovy"),
                expected_language_id: "groovy",
                expected_extensions: &[".mock"],
            },
            LanguageSupportTestCase {
                name: "java language support",
                language_support: MockLanguageSupport::new("java"),
                expected_language_id: "java",
                expected_extensions: &[".mock"],
            },
            LanguageSupportTestCase {
                name: "language with local resolution",
                language_support: MockLanguageSupport::new("test").with_local_resolution(),
                expected_language_id: "test",
                expected_extensions: &[".mock"],
            },
        ];

        for test_case in test_cases {
            // Test language_id
            assert_eq!(
                test_case.language_support.language_id(),
                test_case.expected_language_id,
                "Test '{}': language_id mismatch",
                test_case.name
            );

            // Test file_extensions
            assert_eq!(
                test_case.language_support.file_extensions(),
                test_case.expected_extensions,
                "Test '{}': file_extensions mismatch",
                test_case.name
            );

            // Test parser creation
            let parser = test_case.language_support.create_parser();
            // Basic test that parser was created
            assert!(
                parser.language().is_none(),
                "Parser should be created without language set"
            );
        }
    }

    #[test]
    fn test_definition_chain_resolution() {
        let test_cases = vec![
            DefinitionChainTestCase {
                name: "resolves locally when local is available",
                language_support: MockLanguageSupport::new("test").with_local_resolution(),
                expected_resolution_level: "local",
            },
            DefinitionChainTestCase {
                name: "falls back to project when local unavailable",
                language_support: MockLanguageSupport::new("test").with_project_resolution(),
                expected_resolution_level: "project",
            },
            DefinitionChainTestCase {
                name: "falls back to workspace when project unavailable",
                language_support: MockLanguageSupport::new("test").with_workspace_resolution(),
                expected_resolution_level: "workspace",
            },
            DefinitionChainTestCase {
                name: "falls back to external when workspace unavailable",
                language_support: MockLanguageSupport::new("test").with_external_resolution(),
                expected_resolution_level: "external",
            },
            DefinitionChainTestCase {
                name: "fails when no resolution available",
                language_support: MockLanguageSupport::new("test"),
                expected_resolution_level: "none",
            },
            DefinitionChainTestCase {
                name: "prefers local over other resolutions",
                language_support: MockLanguageSupport::new("test")
                    .with_local_resolution()
                    .with_project_resolution()
                    .with_workspace_resolution()
                    .with_external_resolution(),
                expected_resolution_level: "local",
            },
        ];

        for test_case in test_cases {
            let source = "mock source code";
            let uri = "file:///test/file.mock";
            let dependency_cache = Arc::new(DependencyCache::new());

            // Create a minimal parser and tree for testing
            let mut parser = Parser::new();
            // Note: In a real test, you'd set the language, but for this mock we'll skip it
            let tree = parser.parse(source, None);

            if let Some(tree) = tree {
                let root_node = tree.root_node();
                let result = test_case.language_support.find_definition_chain(
                    &tree,
                    source,
                    dependency_cache,
                    uri,
                    &root_node,
                );

                match test_case.expected_resolution_level {
                    "none" => {
                        assert!(
                            result.is_err(),
                            "Test '{}': expected failure, got success",
                            test_case.name
                        );
                    }
                    _ => {
                        assert!(
                            result.is_ok(),
                            "Test '{}': expected success, got failure: {:?}",
                            test_case.name,
                            result.err()
                        );
                    }
                }
            }
        }
    }

    struct SymbolTypeTestCase {
        name: &'static str,
        symbol_type: SymbolType,
        expected_success: bool,
    }

    #[test]
    fn test_symbol_type_determination() {
        let test_cases = vec![
            SymbolTypeTestCase {
                name: "class declaration symbol type",
                symbol_type: SymbolType::ClassDeclaration,
                expected_success: true,
            },
            SymbolTypeTestCase {
                name: "method call symbol type",
                symbol_type: SymbolType::MethodCall,
                expected_success: true,
            },
            SymbolTypeTestCase {
                name: "variable usage symbol type",
                symbol_type: SymbolType::VariableUsage,
                expected_success: true,
            },
            SymbolTypeTestCase {
                name: "type symbol type",
                symbol_type: SymbolType::Type,
                expected_success: true,
            },
        ];

        for test_case in test_cases {
            let language_support =
                MockLanguageSupport::new("test").with_symbol_type(test_case.symbol_type.clone());

            let source = "mock source";
            let mut parser = Parser::new();
            let tree = parser.parse(source, None);

            if let Some(tree) = tree {
                let root_node = tree.root_node();
                let result =
                    language_support.determine_symbol_type_from_context(&tree, &root_node, source);

                if test_case.expected_success {
                    assert!(
                        result.is_ok(),
                        "Test '{}': expected success, got error: {:?}",
                        test_case.name,
                        result.err()
                    );

                    if let Ok(symbol_type) = result {
                        assert_eq!(
                            symbol_type, test_case.symbol_type,
                            "Test '{}': symbol type mismatch",
                            test_case.name
                        );
                    }
                } else {
                    assert!(
                        result.is_err(),
                        "Test '{}': expected failure, got success",
                        test_case.name
                    );
                }
            }
        }
    }

    #[test]
    fn test_diagnostics_collection() {
        let language_support = MockLanguageSupport::new("test");
        let source = "mock source";
        let mut parser = Parser::new();
        let tree = parser.parse(source, None);

        if let Some(tree) = tree {
            let diagnostics = language_support.collect_diagnostics(&tree, source);
            assert_eq!(diagnostics.len(), 0, "Mock should return no diagnostics");
        }
    }

    #[test]
    fn test_hover_provision() {
        let language_support = MockLanguageSupport::new("test");
        let source = "mock source";
        let location = Location {
            uri: Url::parse("file:///test/file.mock").unwrap(),
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 10,
                },
            },
        };

        let mut parser = Parser::new();
        let tree = parser.parse(source, None);

        if let Some(tree) = tree {
            let hover = language_support.provide_hover(&tree, source, location);
            assert!(hover.is_none(), "Mock should return no hover info");
        }
    }

    #[test]
    fn test_find_implementation() {
        let language_support = MockLanguageSupport::new("test");
        let source = "mock source";
        let position = Position {
            line: 0,
            character: 0,
        };
        let dependency_cache = Arc::new(DependencyCache::new());

        let mut parser = Parser::new();
        let tree = parser.parse(source, None);

        if let Some(tree) = tree {
            let result =
                language_support.find_implementation(&tree, source, position, dependency_cache);

            assert!(result.is_ok(), "Mock implementation should succeed");
            if let Ok(locations) = result {
                assert_eq!(locations.len(), 0, "Mock should return no implementations");
            }
        }
    }

    struct ResolutionMethodTestCase {
        name: &'static str,
        method_name: &'static str,
        language_support: MockLanguageSupport,
        expected_success: bool,
    }

    #[test]
    fn test_individual_resolution_methods() {
        let test_cases = vec![
            ResolutionMethodTestCase {
                name: "find_local returns None by default",
                method_name: "find_local",
                language_support: MockLanguageSupport::new("test"),
                expected_success: false,
            },
            ResolutionMethodTestCase {
                name: "find_local returns Some when enabled",
                method_name: "find_local",
                language_support: MockLanguageSupport::new("test").with_local_resolution(),
                expected_success: true,
            },
            ResolutionMethodTestCase {
                name: "find_in_project returns None by default",
                method_name: "find_in_project",
                language_support: MockLanguageSupport::new("test"),
                expected_success: false,
            },
            ResolutionMethodTestCase {
                name: "find_in_project returns Some when enabled",
                method_name: "find_in_project",
                language_support: MockLanguageSupport::new("test").with_project_resolution(),
                expected_success: true,
            },
        ];

        for test_case in test_cases {
            let source = "mock source";
            let uri = "file:///test/file.mock";
            let dependency_cache = Arc::new(DependencyCache::new());

            let mut parser = Parser::new();
            let tree = parser.parse(source, None);

            if let Some(tree) = tree {
                let root_node = tree.root_node();

                let result = match test_case.method_name {
                    "find_local" => test_case
                        .language_support
                        .find_local(&tree, source, uri, &root_node),
                    "find_in_project" => test_case.language_support.find_in_project(
                        source,
                        uri,
                        &root_node,
                        dependency_cache.clone(),
                    ),
                    "find_in_workspace" => test_case.language_support.find_in_workspace(
                        source,
                        uri,
                        &root_node,
                        dependency_cache.clone(),
                    ),
                    "find_external" => test_case.language_support.find_external(
                        source,
                        uri,
                        &root_node,
                        dependency_cache,
                    ),
                    _ => None,
                };

                assert_eq!(
                    result.is_some(),
                    test_case.expected_success,
                    "Test '{}': expected success = {}, got success = {}",
                    test_case.name,
                    test_case.expected_success,
                    result.is_some()
                );
            }
        }
    }
}
