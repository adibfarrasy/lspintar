use core::panic;
use std::sync::Arc;

use anyhow::{Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::constants::LSP_NAME;
use crate::core::queries::QueryProvider;
use crate::core::{
    dependency_cache::DependencyCache,
    symbols::SymbolType,
    utils::{uri_to_path, find_project_root, path_to_file_uri},
};
use crate::languages::groovy::definition::method_resolution::extract_call_signature_from_context;
use crate::languages::groovy::definition::utils::{get_wildcard_imports_from_source, prepare_symbol_lookup_key_with_wildcard_support, resolve_variable_type, search_definition_in_project, search_static_method_definition_in_project};
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

impl QueryProvider for GroovySupport {
    fn variable_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(variable_declaration) @decl"#,
            r#"(local_variable_declaration) @local_decl"#,
            r#"(expression_statement (identifier) @bare_id)"#,
        ]
    }

    fn method_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(method_declaration) @method"#,
            r#"(constructor_declaration) @constructor"#,
        ]
    }

    fn class_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(class_declaration) @class"#,
            r#"(enum_declaration) @enum"#,
        ]
    }

    fn interface_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(interface_declaration) @interface"#,
        ]
    }

    fn parameter_queries(&self) -> &[&'static str] {
        &[
            r#"(formal_parameter (identifier) @param)"#,
        ]
    }

    fn field_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(field_declaration) @field"#,
            r#"(property_declaration) @property"#,
        ]
    }

    fn symbol_type_detection_query(&self) -> &'static str {
        r#"
        ; DECLARATIONS
        ; Variable declarations
        (variable_declaration
          declarator: (variable_declarator
            name: (identifier) @var_decl))
        ; Field declarations  
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_decl))
        ; Method declarations
        (method_declaration
          name: (identifier) @method_decl)
        ; Class declarations
        (class_declaration
          name: (identifier) @class_decl)
        ; Interface declarations
        (interface_declaration
          name: (identifier) @interface_decl)

        ; USAGES
        ; Method calls
        (method_invocation
          name: (identifier) @method_call)
        ; Field access
        (field_access
          field: (identifier) @field_usage)
        ; Variable usage
        (identifier) @variable_usage
        "#
    }

    fn import_queries(&self) -> &[&'static str] {
        &[
            r#"(import_declaration) @import"#,
        ]
    }

    fn package_queries(&self) -> &[&'static str] {
        &[
            r#"(package_declaration) @package"#,
        ]
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
            
        ; Annotated field declarations (Spring @Autowired, etc.)
        (field_declaration
          (modifiers)? 
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

        ; USAGES - WORKING PATTERNS BASED ON ACTUAL GROOVY GRAMMAR
        ; Object.method() calls - matches object and method name
        (method_invocation 
          object: (identifier) @method_object
          name: (identifier) @method_name)
          
        ; Simple method() calls without object
        (method_invocation 
          name: (identifier) @simple_method_name)


        ; Method usage
        (scoped_identifier) @method_usage
        
        (field_access field: (identifier) @field_usage)

        (method_invocation name: (identifier) @method_usage)
        
        ; Field usage in method invocation objects (service.method())
        (method_invocation
          object: (identifier) @field_usage)

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
        
        ; Standalone identifiers that could be fields (when not in specific contexts) - MUST BE LAST
        (identifier) @potential_field_usage
    "#;

        let query = Query::new(&tree_sitter_groovy::language(), query_text)
            .context("[determine_symbol_type_from_context] failed to create query")?;

        let mut cursor = QueryCursor::new();

        let mut found = false;

        let mut result = Ok(SymbolType::Type); // Default to Type for unmatched contexts
        let mut any_captures_found = false;

        cursor
            .matches(&query, tree.root_node(), source.as_bytes())
            .for_each(|query_match| {
                if found {
                    return;
                }

                for capture in query_match.captures {
                    any_captures_found = true;
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

                            "method_object" => {
                                if self.is_imported_class(capture_text, source) {
                                    SymbolType::Type
                                } else if capture_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                                    SymbolType::Type
                                } else {
                                    SymbolType::VariableUsage
                                }
                            }
                            "method_name" => {
                                SymbolType::MethodCall
                            }
                            "simple_method_name" => {
                                SymbolType::MethodCall
                            }
                            "method_usage" => SymbolType::MethodCall,
                            "type_name" => SymbolType::Type,
                            "super_interface" => SymbolType::SuperInterface,
                            "super_class" => SymbolType::SuperClass,
                            "field_usage" => SymbolType::FieldUsage,
                            "var_usage" => SymbolType::VariableUsage,
                            "potential_field_usage" => {
                                if capture_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                                    SymbolType::Type
                                } else {
                                    SymbolType::VariableUsage
                                }
                            },
                            

                            _ => SymbolType::VariableUsage,
                        };

                        result = Ok(symbol);
                        found = true;
                    }
                }
            });

        if !any_captures_found {
            debug!("determine_symbol_type_from_context: NO CAPTURES FOUND for '{}' - query may be invalid", node_text);
        }

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
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                find_in_project(source, file_uri, usage_node, dependency_cache, self)
            )
        })
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
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                find_external(source, file_uri, usage_node, dependency_cache)
            )
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
        // Use the common method resolution logic that handles static/instance method calls
        crate::languages::common::method_resolution::find_definition_chain_with_method_resolution(
            self, tree, source, dependency_cache, file_uri, usage_node
        )
    }

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
        // Use Groovy-specific logic (reusing existing implementation)
        self.find_static_method_definition_impl(tree, source, file_uri, usage_node, class_name, method_name, dependency_cache)
    }

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
        // Use Groovy-specific logic (reusing existing implementation)
        self.find_instance_method_definition_impl(tree, source, file_uri, usage_node, variable_name, method_name, dependency_cache)
    }

    fn find_method_with_signature<'a>(
        &self,
        tree: &'a Tree,
        source: &str,
        method_name: &str,
        call_signature: &crate::languages::common::method_resolution::CallSignature,
    ) -> Option<tree_sitter::Node<'a>> {
        let result = crate::languages::groovy::definition::method_resolution::find_method_with_signature(
            tree, source, method_name, call_signature
        );
        result
    }

    fn find_field_declaration_type(&self, field_name: &str, tree: &Tree, source: &str) -> Option<String> {
        
        let query_text = r#"
            ; Field declaration with modifiers
            (field_declaration 
              (modifiers)
              type: (type_identifier) @field_type
              declarator: (variable_declarator 
                name: (identifier) @field_name))
                
            ; Field declaration without modifiers
            (field_declaration 
              type: (type_identifier) @field_type
              declarator: (variable_declarator 
                name: (identifier) @field_name))
                
            ; Generic field declaration with modifiers
            (field_declaration 
              (modifiers)
              type: (generic_type 
                (type_identifier) @generic_field_type)
              declarator: (variable_declarator 
                name: (identifier) @generic_field_name))
                
            ; Generic field declaration without modifiers
            (field_declaration 
              type: (generic_type 
                (type_identifier) @generic_field_type)
              declarator: (variable_declarator 
                name: (identifier) @generic_field_name))
        "#;
        
        let language = tree_sitter_groovy::language();
        let query = match tree_sitter::Query::new(&language, query_text) {
            Ok(q) => q,
            Err(e) => {
                return None;
            }
        };
        
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        
        let mut match_count = 0;
        
        while let Some(query_match) = matches.next() {
            match_count += 1;
            
            let mut found_field_name = false;
            let mut field_type = None;
            
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                    
                    match capture_name {
                        "field_name" | "generic_field_name" => {
                            if node_text == field_name {
                                found_field_name = true;
                            }
                        }
                        "field_type" | "generic_field_type" => {
                            field_type = Some(node_text.to_string());
                        }
                        _ => {}
                    }
                }
            }
            
            if found_field_name && field_type.is_some() {
                return field_type;
            }
        }
        
        None
    }
    
    fn find_variable_declaration_type(&self, variable_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        
        let query_text = r#"
            (local_variable_declaration 
              type: (type_identifier) @var_type
              declarator: (variable_declarator 
                name: (identifier) @var_name))
        "#;
        
        let language = tree_sitter_groovy::language();
        let query = tree_sitter::Query::new(&language, query_text).ok()?;
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        
        while let Some(query_match) = matches.next() {
            let mut found_var_name = false;
            let mut var_type = None;
            
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                    match capture_name {
                        "var_name" => {
                            if node_text == variable_name {
                                found_var_name = true;
                            }
                        }
                        "var_type" => {
                            var_type = Some(node_text.to_string());
                        }
                        _ => {}
                    }
                }
            }
            
            if found_var_name && var_type.is_some() {
                return var_type;
            }
        }
        
        None
    }
    
    fn find_parameter_type(&self, param_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        
        let query_text = r#"
            (formal_parameter
              type: (type_identifier) @param_type
              name: (identifier) @param_name)
        "#;
        
        let language = tree_sitter_groovy::language();
        let query = tree_sitter::Query::new(&language, query_text).ok()?;
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        
        while let Some(query_match) = matches.next() {
            let mut found_param_name = false;
            let mut param_type = None;
            
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                    match capture_name {
                        "param_name" => {
                            if node_text == param_name {
                                found_param_name = true;
                            }
                        }
                        "param_type" => {
                            param_type = Some(node_text.to_string());
                        }
                        _ => {}
                    }
                }
            }
            
            if found_param_name && param_type.is_some() {
                return param_type;
            }
        }
        
        None
    }
}

impl GroovySupport {
    /// Implementation of static method resolution (renamed from original method)
    fn find_static_method_definition_impl(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        class_name: &str,
        method_name: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        debug!(
            "find_static_method_definition_impl: looking for {}.{}",
            class_name, method_name
        );

        // First, try to find the class definition using existing resolution chain
        // We create a fake node with the class name for the search

        // Find the class first using the standard resolution chain
        let class_location = self
            .find_local(tree, source, file_uri, usage_node)
            .or_else(|| {
                self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_external(source, file_uri, usage_node, dependency_cache.clone())
            });

        if let Some(location) = class_location {
            debug!(
                "find_static_method_definition_impl: found class {} at {:?}",
                class_name, location.uri
            );
            
            // TODO: Now search for the method within the class file
            // For now, return the class location as a placeholder
            return Some(location);
        }

        debug!(
            "find_static_method_definition_impl: could not find class {}",
            class_name
        );
        None
    }

    /// Implementation of instance method resolution (renamed from original method)
    fn find_instance_method_definition_impl(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        variable_name: &str,
        method_name: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        debug!(
            "find_instance_method_definition_impl: looking for {}.{}",
            variable_name, method_name
        );

        // First, find the variable declaration to determine its type
        let variable_location = self
            .find_local(tree, source, file_uri, usage_node)
            .or_else(|| {
                self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            });

        if let Some(location) = variable_location {
            debug!(
                "find_instance_method_definition_impl: found variable {} at {:?}",
                variable_name, location.uri
            );
            
            // TODO: Determine the type of the variable and then search for the method
            // For now, return the variable location as a placeholder
            return Some(location);
        }

        debug!(
            "find_instance_method_definition_impl: could not find variable {}",
            variable_name
        );
        None
    }

    /// Check if an identifier is an imported class name
    fn is_imported_class(&self, class_name: &str, source: &str) -> bool {
        if self.has_specific_import(class_name, source) {
            return true;
        }
        
        if self.has_wildcard_import_for_class(class_name, source) {
            return true;
        }
        
        false
    }
    
    /// Check if there's a specific import for this class name
    fn has_specific_import(&self, class_name: &str, source: &str) -> bool {
        let query_text = r#"
            (import_declaration) @import_decl
        "#;
        
        let query = tree_sitter::Query::new(&tree_sitter_groovy::language(), query_text);
        if query.is_err() {
            return false;
        }
        let query = query.unwrap();
        
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&tree_sitter_groovy::language()).is_err() {
            return false;
        }
        
        let tree = parser.parse(source, None);
        if tree.is_none() {
            return false;
        }
        let tree = tree.unwrap();
        
        let mut cursor = tree_sitter::QueryCursor::new();
        
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(import_text) = capture.node.utf8_text(source.as_bytes()) {
                    let import_path = import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    if import_path.ends_with(&format!(".{}", class_name)) || import_path == class_name {
                        return true;
                    }
                }
            }
        }
        
        false
    }
    
    /// Check if there's a wildcard import that could include this class
    fn has_wildcard_import_for_class(&self, class_name: &str, source: &str) -> bool {
        // For now, we'll be conservative and assume uppercase class names in wildcard imports are likely classes
        // This could be enhanced by checking against the symbol index
        if let Some(wildcard_packages) = get_wildcard_imports_from_source(source) {
            // If there are wildcard imports and this looks like a class name (uppercase), assume it could be imported
            return !wildcard_packages.is_empty() && 
                   class_name.chars().next().map_or(false, |c| c.is_uppercase());
        }
        false
    }

    /// Specialized resolution for static method calls
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
        
        let class_node = self.create_temporary_class_node(tree, source, class_name);
        if class_node.is_none() {
            return None;
        }
        let class_node = class_node?;
        
        let call_signature = extract_call_signature_from_context(usage_node, source);
        
        
        let class_location = if let Some((project_root, fqn)) = prepare_symbol_lookup_key_with_wildcard_support(
            &class_node, source, file_uri, None, &dependency_cache
        ) {
            
            
            let symbol_key = (project_root.clone(), fqn.clone());
            
            if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                let class_uri = path_to_file_uri(&file_location)?;
                Some(tower_lsp::lsp_types::Location {
                    uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                    range: tower_lsp::lsp_types::Range::default(),
                })
            } else {
                
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
                    
                    let symbol_key = (other_project.clone(), fqn.clone());
                    if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                        let class_uri = path_to_file_uri(&file_location)?;
                        return Some(tower_lsp::lsp_types::Location {
                            uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                            range: tower_lsp::lsp_types::Range::default(),
                        });
                    }
                }
                None
            }
        } else {
            None
        }.or_else(|| {
            self.find_local(tree, source, file_uri, &class_node)
                .or_else(|| {
                    self.find_in_project(source, file_uri, &class_node, dependency_cache.clone())
                })
                .or_else(|| {
                    self.find_in_workspace(source, file_uri, &class_node, dependency_cache.clone())
                })
                .or_else(|| {
                    self.find_external(source, file_uri, &class_node, dependency_cache.clone())
                })
        });
            
        if class_location.is_none() {
            return None;
        }
        let class_location = class_location?;

        let method_location = if let Some(call_sig) = call_signature {
            search_static_method_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        } else {
            search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        };
        
        
        method_location
    }

    /// Create a temporary node for class name resolution
    fn create_temporary_class_node<'a>(&self, tree: &'a Tree, source: &str, class_name: &str) -> Option<Node<'a>> {
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
        
        let variable_type = resolve_variable_type(variable_name, tree, source, usage_node);
        if variable_type.is_none() {
            return None;
        }
        let variable_type = variable_type.unwrap();
        
        let call_signature = extract_call_signature_from_context(usage_node, source);
        
        let class_location = self.resolve_class_through_standard_chain(&variable_type, tree, source, file_uri, dependency_cache.clone());

        if class_location.is_none() {
            return None;
        }
        let class_location = class_location.unwrap();

        
        let result = search_definition_in_project(
            file_uri,
            source,
            usage_node,
            &class_location.uri.to_string(),
            self
        );
        
        let result = if result.is_none() && call_signature.is_some() {
            search_static_method_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        } else {
            result
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
        
        let current_file_path = uri_to_path(file_uri)?;
        let project_root = find_project_root(&current_file_path)?;
        
        
        let direct_symbol_key = (project_root.clone(), class_name.to_string());
        if let Some(file_location) = dependency_cache.symbol_index.get(&direct_symbol_key) {
            let class_uri = path_to_file_uri(&file_location)?;
            return Some(Location {
                uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                range: tower_lsp::lsp_types::Range::default(),
            });
        }
        
        for entry in dependency_cache.symbol_index.iter() {
            let (entry_project_root, fqn) = entry.key();
            if fqn.ends_with(&format!(".{}", class_name)) || fqn == class_name {
                let class_uri = path_to_file_uri(entry.value())?;
                return Some(Location {
                    uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                    range: tower_lsp::lsp_types::Range::default(),
                });
            }
        }
        
        if let Some(mock_node) = self.find_identifier_node_in_tree(tree, source, class_name) {
            if let Some((_, fqn)) = prepare_symbol_lookup_key_with_wildcard_support(
                &mock_node, source, file_uri, Some(project_root.clone()), &dependency_cache
            ) {
                
                let symbol_key = (project_root.clone(), fqn.clone());
                if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                    let class_uri = path_to_file_uri(&file_location)?;
                    return Some(Location {
                        uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                        range: tower_lsp::lsp_types::Range::default(),
                    });
                } else {
                }
                
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
            } else {
            }
        } else {
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

    /// Sequential cross-file resolution to avoid race conditions with dependency cache
    fn find_cross_file_sequential(
        &self,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
        
        if symbol_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            if let Some(location) = self.find_in_project(source, file_uri, usage_node, dependency_cache.clone()) {
                // If the definition is in the same file, don't call set_start_position 
                // as it may find the wrong identifier with the same name
                if location.uri.to_string() == file_uri {
                    return Some(location);
                } else {
                    let uri_string = location.uri.to_string();
                    // Skip set_start_position for builtin sources as they are already correctly positioned
                    if uri_string.contains("lspintar_builtin_sources") {
                        return Some(location);
                    } else {
                        return self.set_start_position(source, usage_node, &uri_string);
                    }
                }
            }
        }
        
        if let Some(location) = self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone()) {
            // If the definition is in the same file, don't call set_start_position 
            // as it may find the wrong identifier with the same name
            if location.uri.to_string() == file_uri {
                return Some(location);
            } else {
                let uri_string = location.uri.to_string();
                // Skip set_start_position for builtin sources as they are already correctly positioned
                if uri_string.contains("lspintar_builtin_sources") {
                    return Some(location);
                } else {
                    return self.set_start_position(source, usage_node, &uri_string);
                }
            }
        }
        
        if let Some(location) = self.find_external(source, file_uri, usage_node, dependency_cache.clone()) {
            // If the definition is in the same file, don't call set_start_position 
            // as it may find the wrong identifier with the same name
            if location.uri.to_string() == file_uri {
                return Some(location);
            } else {
                let uri_string = location.uri.to_string();
                // Skip set_start_position for builtin sources as they are already correctly positioned
                if uri_string.contains("lspintar_builtin_sources") {
                    return Some(location);
                } else {
                    return self.set_start_position(source, usage_node, &uri_string);
                }
            }
        }
        
        if let Some(location) = self.find_in_project(source, file_uri, usage_node, dependency_cache) {
            // If the definition is in the same file, don't call set_start_position 
            // as it may find the wrong identifier with the same name
            if location.uri.to_string() == file_uri {
                return Some(location);
            } else {
                let uri_string = location.uri.to_string();
                // Skip set_start_position for builtin sources as they are already correctly positioned
                if uri_string.contains("lspintar_builtin_sources") {
                    return Some(location);
                } else {
                    return self.set_start_position(source, usage_node, &uri_string);
                }
            }
        }
        
        None
    }

}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
