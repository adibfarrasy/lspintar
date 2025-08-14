use core::panic;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::constants::LSP_NAME;
use crate::core::dependency_cache::DependencyCache;
use crate::core::symbols::SymbolType;
use crate::languages::traits::LanguageSupport;

use super::definition::external::find_external;
use super::definition::local::find_local;
use super::definition::project::find_in_project;
use super::definition::utils::set_start_position;
use super::definition::workspace::find_in_workspace;
use super::diagnostics::collect_syntax_errors;
use super::hover;
use super::implementation;
use super::utils::find_identifier_at_position;

pub struct GroovySupport;

impl GroovySupport {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageSupport for GroovySupport {
    fn language_id(&self) -> &'static str {
        "groovy"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &[".groovy", ".gradle", ".gvy", ".gy", ".gsh"]
    }

    fn create_parser(&self) -> Parser {
        let mut parser = Parser::new();
        if let Err(e) = parser.set_language(&tree_sitter_groovy::language()) {
            eprintln!("Warning: Failed to load Groovy grammar: {:?}", e);
            panic!("cannot load groovy grammar")
        }
        parser
    }

    #[tracing::instrument(skip_all)]
    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic> {
        // TODO: replace this with more sophisticated handling
        collect_syntax_errors(tree, source, LSP_NAME)
    }

    #[tracing::instrument(skip_all)]
    fn find_definition(
        &self,
        tree: &Tree,
        source: &str,
        position: Position,
        uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Location> {
        let identifier_node = find_identifier_at_position(tree, source, position)?;

        self.find_definition_chain(tree, source, dependency_cache, uri, &identifier_node)
    }

    #[tracing::instrument(skip_all)]
    fn find_implementation(
        &self,
        tree: &Tree,
        source: &str,
        position: tower_lsp::lsp_types::Position,
        dependency_cache: Arc<DependencyCache>,
    ) -> Result<Vec<Location>> {
        implementation::handle(tree, source, position, dependency_cache, self)
    }

    #[tracing::instrument(skip_all)]
    fn provide_hover(&self, tree: &Tree, source: &str, location: Location) -> Option<Hover> {
        hover::handle(tree, source, location, self)
    }

    #[tracing::instrument(skip_all)]
    fn determine_symbol_type_from_context(
        &self,
        tree: &Tree,
        node: &Node,
        source: &str,
    ) -> Result<SymbolType> {
        let node_text = node.utf8_text(source.as_bytes())?;

        let query_text = r#"
        ; DECLARATIONS
        ; Variable declarations
        (variable_declaration
          declarator: (variable_declarator
            name: (identifier) @var_decl))

        ; Field declarations  
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_decl))

        ; Class declarations
        (class_declaration
          name: (identifier) @class_decl)

        ; Interface declarations
        (interface_declaration
          name: (identifier) @interface_decl)

        ; Method declarations
        (method_declaration
          name: (identifier) @method_decl)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_decl)

        ; Parameters
        (formal_parameter
          name: (identifier) @param_decl)

        ; USAGES
        (field_access field: (identifier) @field_usage)

        (method_invocation name: (identifier) @method_usage)

        (argument_list (identifier) @var_usage)

        (assignment_expression left: (identifier) @var_usage)

        (assignment_expression right: (identifier) @var_usage)

        ; Interface
        (class_declaration
          interfaces: (super_interfaces
            (type_list (type_identifier) @super_interface)))
        (interface_declaration
          (extends_interfaces
            (type_list (type_identifier) @super_interface)))

        ; Superclass
        (class_declaration
          superclass: (superclass
            (type_identifier) @super_class))

        ; Type identifiers
        (type_identifier) @type_name

        ; Imports
        (import_declaration
          (scoped_identifier) @import_name) 

        ; Method usage
        (scoped_identifier) @method_usage
    "#;

        let query = Query::new(&tree_sitter_groovy::language(), query_text)
            .context("[determine_symbol_type_from_context] failed to create query")?;

        let mut cursor = QueryCursor::new();

        let mut found = false;

        let mut result = Ok(SymbolType::Type); // Default to Type for unmatched contexts

        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|query_match| {
                if found {
                    return;
                }

                for capture in query_match.captures {
                    let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap();

                    let capture_range = capture.node.range();
                    let node_range = node.range();

                    if capture_text == node_text && capture_range == node_range {
                        let capture_name = query.capture_names()[capture.index as usize];
                        let symbol = match capture_name {
                            "import_name" => SymbolType::PackageDeclaration,
                            "var_decl" => SymbolType::VariableDeclaration,
                            "field_decl" => SymbolType::FieldDeclaration,
                            "class_decl" => SymbolType::ClassDeclaration,
                            "interface_decl" => SymbolType::InterfaceDeclaration,
                            "method_decl" => SymbolType::MethodDeclaration,
                            "enum_decl" => SymbolType::EnumDeclaration,
                            "param_decl" => SymbolType::ParameterDeclaration,

                            "method_usage" => SymbolType::MethodCall,
                            "type_name" => SymbolType::Type,
                            "super_interface" => SymbolType::SuperInterface,
                            "super_class" => SymbolType::SuperClass,
                            "field_usage" => SymbolType::FieldUsage,
                            "var_usage" => SymbolType::VariableUsage,

                            _ => SymbolType::VariableUsage,
                        };

                        result = Ok(symbol);
                        found = true;
                    }
                }
            });

        debug!("node_text: {node_text}, result: {:#?}", result);
        result
    }

    fn find_local(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
    ) -> Option<Location> {
        find_local(tree, source, file_uri, usage_node, self)
    }

    fn find_in_project(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        find_in_project(source, file_uri, usage_node, dependency_cache, self)
    }

    fn find_in_workspace(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        find_in_workspace(source, file_uri, usage_node, dependency_cache, self)
    }

    fn find_external(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        find_external(source, file_uri, usage_node, dependency_cache)
    }

    fn set_start_position(
        &self,
        source: &str,
        usage_node: &Node,
        file_uri: &str,
    ) -> Option<Location> {
        set_start_position(source, usage_node, file_uri)
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
