use std::sync::Arc;

use anyhow::{anyhow, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tree_sitter::{Node, Parser, Tree, Query, QueryCursor, StreamingIterator};
use tracing::warn;

use crate::core::queries::QueryProvider;
use crate::core::{dependency_cache::DependencyCache, symbols::SymbolType};
use crate::languages::traits::LanguageSupport;

use super::definition::{external, local, project, workspace};
use super::diagnostics::collect_syntax_errors;
use super::hover;
use super::implementation;
use super::utils::find_identifier_at_position;
use super::definition::utils::set_start_position;

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
            r#"(object_declaration) @object"#,
        ]
    }

    fn interface_declaration_queries(&self) -> &[&'static str] {
        &[r#"(class_declaration) @interface"#]
    }

    fn parameter_queries(&self) -> &[&'static str] {
        &[
            r#"(parameter (simple_identifier) @param)"#,
            r#"(class_parameter (simple_identifier) @param)"#,
        ]
    }

    fn field_declaration_queries(&self) -> &[&'static str] {
        &[r#"(property_declaration) @property"#]
    }

    fn symbol_type_detection_query(&self) -> &'static str {
        r#"
        ; DECLARATIONS
        ; Property declarations (val/var)
        (property_declaration
          (variable_declaration
            (simple_identifier) @var_decl))
            
        ; Function declarations
        (function_declaration
          (simple_identifier) @method_decl)
          
        ; Class declarations (including enum classes)
        (class_declaration
          (type_identifier) @class_decl)
          
        ; Object declarations
        (object_declaration
          (type_identifier) @object_decl)

        ; Parameters
        (parameter
          (simple_identifier) @param_decl)
        (class_parameter
          (simple_identifier) @param_decl)

        ; USAGES
        ; Call expressions
        (call_expression
          (simple_identifier) @method_call)
        (call_expression
          (navigation_expression
            (navigation_suffix
              (simple_identifier) @method_call)))
              
        ; Navigation expressions (property access)
        (navigation_expression
          (navigation_suffix
            (simple_identifier) @field_usage))
            
        ; Type identifiers
        (type_identifier) @type_name
        
        ; Simple identifiers (variables)
        (simple_identifier) @variable_usage
        "#
    }

    fn import_queries(&self) -> &[&'static str] {
        &[r#"(import_header) @import"#]
    }

    fn package_queries(&self) -> &[&'static str] {
        &[r#"(package_header) @package"#]
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
        if let Err(e) = parser.set_language(&tree_sitter_kotlin::language()) {
            eprintln!("Warning: Failed to load Kotlin grammar: {:?}", e);
            panic!("cannot load kotlin grammar")
        }
        parser
    }

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic> {
        collect_syntax_errors(tree, source, "kotlin-lsp")
    }

    fn find_definition(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {

        if let Some(identifier_node) = find_identifier_at_position(tree, source, position) {
            let identifier_text = identifier_node.utf8_text(source.as_bytes()).unwrap_or("?");

            let result = self.find_definition_chain(tree, source, dependency_cache, uri, &identifier_node);
            if let Err(e) = &result {
                warn!("Kotlin: Failed to find definition for '{}': {:?}", identifier_text, e);
            }
            result
        } else {
            warn!("Kotlin: No identifier found at position {:?} in {}", position, uri);
            let root_node = tree.root_node();
            self.find_definition_chain(tree, source, dependency_cache, uri, &root_node)
        }
    }

    fn find_implementation(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>> {
        implementation::handle(tree, source, position, dependency_cache, self)
    }

    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover> {
        hover::handle(tree, source, location, self)
    }

    fn determine_symbol_type_from_context(
        &self,
        tree: &Tree,
        node: &Node,
        source: &str,
    ) -> Result<SymbolType> {
        let node_text = node.utf8_text(source.as_bytes())?;

        let query_text = self.symbol_type_detection_query();
        let query = Query::new(&tree_sitter_kotlin::language(), query_text)
            .map_err(|e| anyhow!("Failed to create Kotlin symbol type detection query: {:?}", e))?;

        let mut cursor = QueryCursor::new();
        let mut found = false;
        let mut result = Ok(SymbolType::Type);

        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(query_match) = matches.next() {
            if found {
                break;
            }

            for capture in query_match.captures {
                let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                let capture_range = capture.node.range();
                let node_range = node.range();

                if capture_text == node_text && capture_range == node_range {
                    let capture_name = query.capture_names()[capture.index as usize];

                    let symbol = match capture_name {
                        "var_decl" => SymbolType::VariableDeclaration,
                        "method_decl" => SymbolType::MethodDeclaration,
                        "class_decl" => SymbolType::ClassDeclaration,
                        "object_decl" => SymbolType::ClassDeclaration,
                        "param_decl" => SymbolType::ParameterDeclaration,
                        "method_call" => SymbolType::MethodCall,
                        "field_usage" => SymbolType::FieldUsage,
                        "type_name" => SymbolType::Type,
                        "variable_usage" => SymbolType::VariableUsage,
                        _ => SymbolType::VariableUsage,
                    };

                    result = Ok(symbol);
                    found = true;
                    break;
                }
            }
        }

        if !found {
        }

        result
    }

    // Use shared generic algorithms for definition resolution
    fn find_local(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
    ) -> Option<Location> {
        local::find_local(tree, source, file_uri, usage_node, self)
    }

    fn find_in_project(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(project::find_in_project(
                source,
                file_uri,
                usage_node,
                dependency_cache,
                self,
            ))
        })
    }

    fn find_in_workspace(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        workspace::find_in_workspace(source, file_uri, usage_node, dependency_cache, self)
    }

    fn find_external(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(external::find_external(
                source,
                file_uri,
                usage_node,
                dependency_cache,
            ))
        })
    }

    fn set_start_position(
        &self,
        source: &str,
        usage_node: &Node,
        file_uri: &str,
    ) -> Option<Location> {
        set_start_position(source, usage_node, file_uri)
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
            .or_else(|| {
                self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| self.find_external(source, file_uri, usage_node, dependency_cache.clone()))
            .and_then(|location| {
                // If the definition is in the same file, don't call set_start_position
                // as it may find the wrong identifier with the same name
                if location.uri.to_string() == file_uri {
                    Some(location)
                } else {
                    let uri_string = location.uri.to_string();
                    // Skip set_start_position for builtin sources as they are already correctly positioned
                    if uri_string.contains("lspintar_builtin_sources") {
                        Some(location)
                    } else {
                        self.set_start_position(source, usage_node, &uri_string)
                    }
                }
            })
            .ok_or_else(|| anyhow!("Definition not found"))
    }
}

impl Default for KotlinSupport {
    fn default() -> Self {
        Self::new()
    }
}
