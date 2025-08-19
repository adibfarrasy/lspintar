use std::sync::Arc;

use anyhow::{anyhow, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Node, Parser, Tree};

use crate::core::{
    dependency_cache::DependencyCache,
    symbols::SymbolType,
    definition::{
        queries::QueryProvider,
        local::find_local_generic,
    },
    cross_language::type_bridge::CrossLanguageTypeInfo,
    registry::LanguageRegistry,
};
use crate::languages::traits::LanguageSupport;

pub struct KotlinSupport;

impl KotlinSupport {
    pub fn new() -> Self {
        Self
    }
}

impl QueryProvider for KotlinSupport {
    fn variable_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(property_declaration) @decl"#,
            r#"(variable_declaration) @local_decl"#,
        ]
    }

    fn method_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(function_declaration) @function"#,
            r#"(primary_constructor) @constructor"#,
            r#"(secondary_constructor) @constructor"#,
        ]
    }

    fn class_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(class_declaration) @class"#,
            r#"(enum_class_declaration) @enum"#,
            r#"(object_declaration) @object"#,
        ]
    }

    fn interface_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(interface_declaration) @interface"#,
        ]
    }

    fn parameter_queries(&self) -> &[&'static str] {
        &[
            r#"(value_parameter (simple_identifier) @param)"#,
        ]
    }

    fn field_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(property_declaration) @property"#,
        ]
    }

    fn symbol_type_detection_query(&self) -> &'static str {
        r#"
        ; DECLARATIONS
        (property_declaration
          name: (simple_identifier) @var_decl)
        (function_declaration
          name: (simple_identifier) @method_decl)
        (class_declaration
          name: (simple_identifier) @class_decl)
        (interface_declaration
          name: (simple_identifier) @interface_decl)

        ; USAGES
        (call_expression
          (simple_identifier) @method_call)
        (navigation_expression
          (simple_identifier) @field_usage)
        (simple_identifier) @variable_usage
        "#
    }

    fn import_queries(&self) -> &[&'static str] {
        &[
            r#"(import_header) @import"#,
        ]
    }

    fn package_queries(&self) -> &[&'static str] {
        &[
            r#"(package_header) @package"#,
        ]
    }
}

impl LanguageSupport for KotlinSupport {
    fn language_id(&self) -> &'static str {
        "kotlin"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &[".kt", ".kts"]
    }

    fn create_parser(&self) -> Parser {
        let mut parser = Parser::new();
        // TODO: Implement Kotlin parser setup when tree-sitter-kotlin is added
        // if let Err(e) = parser.set_language(&tree_sitter_kotlin::language()) {
        //     eprintln!("Warning: Failed to load Kotlin grammar: {:?}", e);
        //     panic!("cannot load kotlin grammar")
        // }
        todo!("Kotlin parser setup not implemented yet - add tree-sitter-kotlin dependency");
        // parser
    }

    fn collect_diagnostics(&self, _tree: &Tree, _source: &str) -> Vec<Diagnostic> {
        // TODO: Implement Kotlin-specific diagnostics
        todo!("Kotlin diagnostics collection not implemented yet");
    }

    fn find_definition(
        &self,
        tree: &Tree,
        source: &str,
        _position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {
        // TODO: Implement Kotlin-specific definition finding using shared algorithms
        // For now, use a placeholder that would work with the generic algorithms
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
        // TODO: Implement Kotlin implementation finding
        todo!("Kotlin implementation finding not implemented yet");
    }

    fn provide_hover(&self, _tree: &Tree, _source: &str, _location: Location) -> Option<Hover> {
        // TODO: Implement Kotlin hover support
        todo!("Kotlin hover support not implemented yet");
    }

    fn determine_symbol_type_from_context(
        &self,
        _tree: &Tree,
        _node: &Node,
        _source: &str,
    ) -> Result<SymbolType> {
        // TODO: Implement Kotlin-specific symbol type detection
        todo!("Kotlin symbol type detection not implemented yet");
    }

    fn extract_type_info(&self, _tree: &Tree, _source: &str, _node: &Node) -> Option<CrossLanguageTypeInfo> {
        // TODO: Implement Kotlin type info extraction for cross-language support
        todo!("Kotlin type info extraction not implemented yet");
    }

    fn find_cross_language_definition(
        &self,
        _symbol: &str,
        _target_language: &str,
        _registry: &LanguageRegistry,
        _dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // TODO: Implement Kotlin cross-language definition finding
        todo!("Kotlin cross-language definition finding not implemented yet");
    }

    // Use shared generic algorithms for definition resolution
    fn find_local(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
    ) -> Option<Location> {
        find_local_generic(tree, source, file_uri, usage_node, self)
    }

    fn find_in_project(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // TODO: Implement Kotlin project definition finding
        None
    }

    fn find_in_workspace(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // TODO: Implement Kotlin workspace definition finding
        None
    }

    fn find_external(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // TODO: Implement Kotlin external definition finding
        None
    }

    fn set_start_position(
        &self,
        _source: &str,
        _usage_node: &Node,
        _file_uri: &str,
    ) -> Option<Location> {
        // TODO: Implement Kotlin-specific position setting
        todo!("Kotlin position setting not implemented yet");
    }

    fn find_definition_chain(
        &self,
        tree: &Tree,
        source: &str,
        dependency_cache: Arc<DependencyCache>,
        file_uri: &str,
        usage_node: &Node,
    ) -> Result<Location> {
        // Use the standard definition resolution chain
        self.find_local(tree, source, file_uri, usage_node)
            .or_else(|| self.find_in_project(source, file_uri, usage_node, dependency_cache.clone()))
            .or_else(|| self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone()))
            .or_else(|| self.find_external(source, file_uri, usage_node, dependency_cache.clone()))
            .and_then(|location| self.set_start_position(source, usage_node, &location.uri.to_string()))
            .ok_or_else(|| anyhow!("Definition not found"))
    }
}

impl Default for KotlinSupport {
    fn default() -> Self {
        Self::new()
    }
}