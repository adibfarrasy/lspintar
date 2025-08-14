use core::panic;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::constants::LSP_NAME;
use crate::core::{
    dependency_cache::DependencyCache,
    symbols::SymbolType,
    utils::{uri_to_path, find_project_root, path_to_file_uri},
};
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

        ; Static method usage (e.g., ClassName.method)
        (method_invocation 
          object: (identifier) @static_method_object
          name: (identifier) @static_method_name)

        ; Instance method usage (e.g., variable.method)  
        (method_invocation 
          object: (identifier) @instance_method_object
          name: (identifier) @instance_method_name)

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

                            "static_method_object" => {
                                // Check if this looks like a class name (uppercase first letter)
                                if capture_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                                    SymbolType::Type // Static method call on class
                                } else {
                                    SymbolType::VariableUsage // Instance method call on variable
                                }
                            }
                            "static_method_name" => SymbolType::MethodCall,
                            "instance_method_object" => {
                                // Check if this looks like a variable name (lowercase first letter)
                                if capture_text.chars().next().map_or(false, |c| c.is_lowercase()) {
                                    SymbolType::VariableUsage // Instance method call on variable
                                } else {
                                    SymbolType::Type // Static method call on class
                                }
                            }
                            "instance_method_name" => SymbolType::MethodCall,
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

    fn find_definition_chain(
        &self,
        tree: &Tree,
        source: &str,
        dependency_cache: Arc<DependencyCache>,
        file_uri: &str,
        usage_node: &Node,
    ) -> Result<Location> {
        // Optimized: Fast-path for common local cases
        let symbol_type = self.determine_symbol_type_from_context(tree, usage_node, source).ok();
        
        // For simple local symbols, try local resolution first (fastest)
        if let Some(symbol_type) = symbol_type {
            if matches!(symbol_type, 
                SymbolType::VariableUsage | 
                SymbolType::ParameterDeclaration |
                SymbolType::MethodCall
            ) {
                if let Some(local_location) = self.find_local(tree, source, file_uri, usage_node) {
                    return Ok(local_location);
                }
            }
        }
        
        // Check if this is a static method call pattern first
        if let Some((class_name, method_name)) = super::definition::utils::extract_static_method_context(usage_node, source) {
            // For static method calls, we need to resolve the class first, then find the method
            if let Some(location) = self.find_static_method_definition(tree, source, file_uri, usage_node, &class_name, &method_name, dependency_cache.clone()) {
                return Ok(location);
            }
        }

        // Check if this is an instance method call pattern
        if let Some((variable_name, method_name)) = super::definition::utils::extract_instance_method_context(usage_node, source) {
            // For instance method calls, we need to resolve the variable type first, then find the method
            if let Some(location) = self.find_instance_method_definition(tree, source, file_uri, usage_node, &variable_name, &method_name, dependency_cache.clone()) {
                return Ok(location);
            }
        }

        // Try local resolution again if not in fast-path
        if let Some(local_location) = self.find_local(tree, source, file_uri, usage_node) {
            return Ok(local_location);
        }
        
        // Continue with cross-file resolution chain
        self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            .or_else(|| {
                self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| self.find_external(source, file_uri, usage_node, dependency_cache.clone()))
            .and_then(|location| {
                self.set_start_position(source, usage_node, &location.uri.to_string())
            })
            .ok_or_else(|| anyhow::anyhow!("Definition not found"))
    }

}

impl GroovySupport {
    /// Specialized resolution for static method calls like ObjectTransferUtil.transferObject()
    fn find_static_method_definition(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        class_name: &str,
        method_name: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // Create a temporary node representing the class name to resolve the class first
        let class_node = self.create_temporary_class_node(tree, source, class_name)?;
        
        // Extract call signature for method matching
        let call_signature = super::definition::method_resolution::extract_call_signature_from_context(usage_node, source);
        
        // Try to resolve the class through the normal resolution chain
        let class_location = self.find_local(tree, source, file_uri, &class_node)
            .or_else(|| {
                self.find_in_project(source, file_uri, &class_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_in_workspace(source, file_uri, &class_node, dependency_cache.clone())
            })
            .or_else(|| self.find_external(source, file_uri, &class_node, dependency_cache.clone()))?;

        // Now search for the method in the resolved class file
        if let Some(call_sig) = call_signature {
            super::definition::utils::search_static_method_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        } else {
            // Fallback to regular method search
            super::definition::utils::search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        }
    }

    /// Create a temporary node for class name resolution
    fn create_temporary_class_node<'a>(&self, tree: &'a Tree, source: &str, class_name: &str) -> Option<Node<'a>> {
        // This is a workaround - we need to find an actual node in the tree that represents the class name
        // In a static method call like ObjectTransferUtil.transferObject(), we can find the ObjectTransferUtil node
        let query_text = r#"
            (method_invocation 
              object: (identifier) @class_name)
        "#;

        let query = Query::new(&tree_sitter_groovy::language(), query_text).ok()?;
        let mut cursor = QueryCursor::new();

        let mut result = None;
        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|query_match| {
                for capture in query_match.captures {
                    let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if node_text == class_name {
                        result = Some(capture.node);
                        return;
                    }
                }
            });

        result
    }

    /// Specialized resolution for instance method calls like variable.method()
    fn find_instance_method_definition(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        variable_name: &str,
        method_name: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        
        // First, resolve the variable to find its type
        let variable_type = super::definition::utils::resolve_variable_type(variable_name, tree, source, usage_node)?;
        
        // Extract call signature for method matching
        let call_signature = super::definition::method_resolution::extract_call_signature_from_context(usage_node, source);
        
        // Create a simple temporary node for the class name and use existing resolution chain
        let class_location = self.resolve_class_through_standard_chain(&variable_type, tree, source, file_uri, dependency_cache.clone());

        if class_location.is_none() {
            return None;
        }
        let class_location = class_location?;

        // Now search for the method in the resolved class file
        let result = if let Some(call_sig) = call_signature {
            super::definition::utils::search_static_method_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        } else {
            // Fallback to regular method search
            super::definition::utils::search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        };


        result
    }

    /// Resolve a class through the standard resolution chain by creating a virtual lookup
    fn resolve_class_through_standard_chain(
        &self,
        class_name: &str,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        
        // Use the existing utility functions to resolve the class name
        // This leverages all the existing import resolution, wildcard resolution, etc.
        let current_file_path = uri_to_path(file_uri)?;
        let project_root = find_project_root(&current_file_path)?;
        
        // Try to resolve using the existing prepare_symbol_lookup_key_with_wildcard_support
        // We create a mock usage node by finding any identifier in the tree with the class name
        if let Some(mock_node) = self.find_identifier_node_in_tree(tree, source, class_name) {
            // Use the existing resolution utilities
            if let Some((_, fqn)) = super::definition::utils::prepare_symbol_lookup_key_with_wildcard_support(
                &mock_node, source, file_uri, Some(project_root.clone()), &dependency_cache
            ) {
                
                // Look up the class in the symbol index
                let symbol_key = (project_root.clone(), fqn.clone());
                if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                    let class_uri = path_to_file_uri(&file_location)?;
                    return Some(Location {
                        uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                        range: tower_lsp::lsp_types::Range::default(),
                    });
                }
                
                // Try workspace projects
                let workspace_projects: Vec<std::path::PathBuf> = dependency_cache
                    .symbol_index
                    .iter()
                    .map(|entry| entry.key().0.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                    
                for other_project in workspace_projects {
                    if other_project == project_root {
                        continue;
                    }
                    
                    let symbol_key = (other_project, fqn.clone());
                    if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                        let class_uri = path_to_file_uri(&file_location)?;
                        return Some(Location {
                            uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                            range: tower_lsp::lsp_types::Range::default(),
                        });
                    }
                }
            }
        }
        
        None
    }

    /// Find an identifier node in the tree that matches the given text
    fn find_identifier_node_in_tree<'a>(&self, tree: &'a Tree, source: &str, identifier: &str) -> Option<Node<'a>> {
        let query_text = r#"(identifier) @name"#;
        let query = Query::new(&tree_sitter_groovy::language(), query_text).ok()?;
        let mut cursor = QueryCursor::new();

        let mut result = None;
        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|query_match| {
                for capture in query_match.captures {
                    let node_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if node_text == identifier {
                        result = Some(capture.node);
                        return;
                    }
                }
            });

        result
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
