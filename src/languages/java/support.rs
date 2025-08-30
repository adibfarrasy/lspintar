use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::warn;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::queries::QueryProvider;
use crate::core::{dependency_cache::DependencyCache, symbols::SymbolType};

use super::definition::{external, local, project, workspace};
use crate::languages::traits::LanguageSupport;

use super::diagnostics::collect_syntax_errors;
use super::hover;
use super::implementation;
use super::utils::find_identifier_at_position;

pub struct JavaSupport;

impl JavaSupport {
    pub fn new() -> Self {
        Self
    }
}

impl QueryProvider for JavaSupport {
    fn variable_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(local_variable_declaration) @local_decl"#,
            r#"(field_declaration) @field_decl"#,
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
        &[r#"(interface_declaration) @interface"#]
    }

    fn parameter_queries(&self) -> &[&'static str] {
        &[r#"(formal_parameter (identifier) @param)"#]
    }

    fn field_declaration_queries(&self) -> &[&'static str] {
        &[r#"(field_declaration) @field"#]
    }

    fn symbol_type_detection_query(&self) -> &'static str {
        r#"
        ; DECLARATIONS
        (local_variable_declaration
          declarator: (variable_declarator
            name: (identifier) @var_decl))
        (field_declaration
          declarator: (variable_declarator
            name: (identifier) @field_decl))
        (method_declaration
          name: (identifier) @method_decl)
        (class_declaration
          name: (identifier) @class_decl)
        (interface_declaration
          name: (identifier) @interface_decl)

        ; USAGES
        (method_invocation
          name: (identifier) @method_call)
        (method_reference
          (identifier) @method_reference)
        (field_access
          field: (identifier) @field_usage)
        (identifier) @variable_usage
        "#
    }

    fn import_queries(&self) -> &[&'static str] {
        &[r#"(import_declaration) @import"#]
    }

    fn package_queries(&self) -> &[&'static str] {
        &[r#"(package_declaration) @package"#]
    }
}

impl LanguageSupport for JavaSupport {
    fn language_id(&self) -> &'static str {
        "java"
    }

    fn file_extensions(&self) -> &[&'static str] {
        &[".java"]
    }

    fn create_parser(&self) -> Parser {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        if let Err(e) = parser.set_language(&language) {
            eprintln!("Warning: Failed to load Java grammar: {:?}", e);
            panic!("cannot load java grammar")
        }
        parser
    }

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic> {
        collect_syntax_errors(tree, source, "java-lsp")
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

            // Try to determine symbol type
            match self.determine_symbol_type_from_context(tree, &identifier_node, source) {
                Ok(symbol_type) => {

                    let result = self.find_definition_chain(
                        tree,
                        source,
                        dependency_cache,
                        uri,
                        &identifier_node,
                    );
                    if let Err(e) = &result {
                        warn!(
                            "Java: Failed to find definition for '{}': {:?}",
                            identifier_text, e
                        );
                    }
                    result
                }
                Err(e) => {
                    warn!(
                        "Java: Failed to determine symbol type for '{}': {:?}",
                        identifier_text, e
                    );
                    Err(e)
                }
            }
        } else {
            let file_path = uri.strip_prefix("file://").unwrap_or(uri);
            warn!("Java: No identifier found at position {:?} in {} - file may not be indexed yet", position, file_path);

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

        let query_text = r#"
        ; DECLARATIONS
        ; Local variable declarations  
        (local_variable_declaration
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

        ; Constructor declarations
        (constructor_declaration
          name: (identifier) @constructor_decl)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_decl)

        ; Parameters
        (formal_parameter
          name: (identifier) @param_decl)

        ; USAGES
        ; Method invocations
        (method_invocation 
          object: (identifier) @method_object
          name: (identifier) @method_name)
          
        ; Simple method() calls without object
        (method_invocation 
          name: (identifier) @simple_method_name)

        ; Field access
        (field_access field: (identifier) @field_usage)

        ; Method usage in various contexts
        (method_invocation name: (identifier) @method_usage)
        
        ; Field usage in method invocation objects
        (method_invocation
          object: (identifier) @field_usage)

        ; Variable usage in assignments and arguments
        (argument_list (identifier) @var_usage)
        (assignment_expression left: (identifier) @var_usage)
        (assignment_expression right: (identifier) @var_usage)

        ; Type identifiers and inheritance
        (class_declaration
          superclass: (superclass (type_identifier) @super_class))

        (class_declaration
          interfaces: (super_interfaces
            (type_list (type_identifier) @super_interface)))

        (interface_declaration
          (extends_interfaces
            (type_list (type_identifier) @super_interface)))

        ; Type identifiers
        (type_identifier) @type_name

        ; Method references (Java 8+ lambda syntax)
        (method_reference
          (identifier) @method_reference)

        ; Imports
        (import_declaration
          (scoped_identifier) @import_name) 
        
        ; Generic identifiers - must be last
        (identifier) @potential_field_usage
    "#;

        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(&language, query_text)
            .context("Failed to create Java symbol type detection query")?;

        let mut cursor = QueryCursor::new();
        let mut found = false;
        let mut result = Ok(SymbolType::Type);

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
                            "constructor_decl" => SymbolType::MethodDeclaration,
                            "enum_decl" => SymbolType::EnumDeclaration,
                            "param_decl" => SymbolType::ParameterDeclaration,

                            "method_object" => {
                                // Check if this is a class name (uppercase) or variable (lowercase)
                                if capture_text
                                    .chars()
                                    .next()
                                    .map_or(false, |c| c.is_uppercase())
                                {
                                    SymbolType::Type
                                } else {
                                    SymbolType::VariableUsage
                                }
                            }
                            "method_name" => SymbolType::MethodCall,
                            "simple_method_name" => SymbolType::MethodCall,
                            "method_usage" => SymbolType::MethodCall,
                            "method_reference" => SymbolType::MethodCall,
                            "type_name" => SymbolType::Type,
                            "super_interface" => SymbolType::SuperInterface,
                            "super_class" => SymbolType::SuperClass,
                            "field_usage" => SymbolType::FieldUsage,
                            "var_usage" => SymbolType::VariableUsage,
                            "potential_field_usage" => {
                                // Default to field usage for unmatched identifiers
                                SymbolType::FieldUsage
                            }

                            _ => SymbolType::VariableUsage,
                        };

                        result = Ok(symbol);
                        found = true;
                    }
                }
            });

        result
    }

    // Use Java-specific definition resolution algorithms
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
        use crate::core::utils::set_start_position_for_language;
        set_start_position_for_language(source, usage_node, file_uri, "java")
    }

    fn find_method_with_signature<'a>(
        &self,
        tree: &'a Tree,
        source: &str,
        method_name: &str,
        call_signature: &crate::languages::common::method_resolution::CallSignature,
    ) -> Option<tree_sitter::Node<'a>> {
        // Convert the common CallSignature to Java's CallSignature
        let java_call_sig = crate::languages::java::definition::method_resolution::CallSignature {
            arg_count: call_signature.arg_count,
            arg_types: call_signature.arg_types.clone(),
        };
        
        crate::languages::java::definition::method_resolution::find_method_with_signature(
            tree, source, method_name, &java_call_sig
        )
    }

    fn find_field_declaration_type(&self, field_name: &str, tree: &Tree, source: &str) -> Option<String> {
        use tracing::debug;
        debug!("JAVA: find_field_declaration_type: looking for field '{}' in source", field_name);
        
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
        
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = match tree_sitter::Query::new(&language, query_text) {
            Ok(q) => q,
            Err(e) => {
                debug!("JAVA: find_field_declaration_type: failed to create query: {:?}", e);
                return None;
            }
        };
        
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        
        debug!("JAVA: find_field_declaration_type: executing tree-sitter query for field declarations");
        let mut match_count = 0;
        
        while let Some(query_match) = matches.next() {
            match_count += 1;
            debug!("JAVA: find_field_declaration_type: processing match #{}", match_count);
            
            let mut found_field_name = false;
            let mut field_type = None;
            
            for capture in query_match.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if let Ok(node_text) = capture.node.utf8_text(source.as_bytes()) {
                    debug!("JAVA: find_field_declaration_type: found capture '{}' = '{}'", capture_name, node_text);
                    
                    match capture_name {
                        "field_name" | "generic_field_name" => {
                            if node_text == field_name {
                                debug!("JAVA: find_field_declaration_type: found matching field name '{}'", field_name);
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
                debug!("JAVA: find_field_declaration_type: successfully found field '{}' with type '{:?}'", field_name, field_type);
                return field_type;
            }
        }
        
        debug!("JAVA: find_field_declaration_type: no field declaration found for '{}' (processed {} matches)", field_name, match_count);
        None
    }
    
    fn find_variable_declaration_type(&self, variable_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        use tracing::debug;
        debug!("JAVA: find_variable_declaration_type: looking for variable '{}' in source", variable_name);
        
        let query_text = r#"
            (local_variable_declaration 
              type: (type_identifier) @var_type
              declarator: (variable_declarator 
                name: (identifier) @var_name))
        "#;
        
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
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
                debug!("JAVA: find_variable_declaration_type: found variable '{}' with type '{:?}'", variable_name, var_type);
                return var_type;
            }
        }
        
        debug!("JAVA: find_variable_declaration_type: no variable declaration found for '{}'", variable_name);
        None
    }
    
    fn find_parameter_type(&self, param_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        use tracing::debug;
        debug!("JAVA: find_parameter_type: looking for parameter '{}' in source", param_name);
        
        let query_text = r#"
            (formal_parameter
              type: (type_identifier) @param_type
              name: (identifier) @param_name)
        "#;
        
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
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
                debug!("JAVA: find_parameter_type: found parameter '{}' with type '{:?}'", param_name, param_type);
                return param_type;
            }
        }
        
        debug!("JAVA: find_parameter_type: no parameter declaration found for '{}'", param_name);
        None
    }

    fn find_definition_chain(
        &self,
        tree: &Tree,
        source: &str,
        dependency_cache: Arc<DependencyCache>,
        file_uri: &str,
        usage_node: &Node,
    ) -> Result<Location> {
        // Use the common method resolution logic that handles static/instance method calls with cross-language support
        crate::languages::common::method_resolution::find_definition_chain_with_method_resolution(
            self, tree, source, dependency_cache, file_uri, usage_node
        )
    }
}

impl Default for JavaSupport {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions
fn extract_package_from_tree(tree: &Tree, source: &str) -> Option<String> {
    let query_text = r#"(package_declaration (scoped_identifier) @package)"#;
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    let query = Query::new(&language, query_text).ok()?;
    let mut cursor = QueryCursor::new();

    let mut result = None;
    cursor
        .matches(&query, tree.root_node(), source.as_bytes())
        .take(1)
        .for_each(|query_match| {
            for capture in query_match.captures {
                result = capture
                    .node
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(String::from);
            }
        });

    result
}

fn check_if_abstract(node: &Node, source: &str) -> bool {
    // Check if a class/method has abstract modifier
    let mut current = Some(*node);
    while let Some(node) = current {
        if node.kind().ends_with("_declaration") {
            // Look for modifiers child
            for child in node.children(&mut node.walk()) {
                if child.kind() == "modifiers" {
                    if let Ok(modifier_text) = child.utf8_text(source.as_bytes()) {
                        if modifier_text.contains("abstract") {
                            return true;
                        }
                    }
                }
            }
            break;
        }
        current = node.parent();
    }
    false
}
