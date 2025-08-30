use std::sync::Arc;

use anyhow::{anyhow, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::{debug, warn};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::core::queries::QueryProvider;
use crate::core::{dependency_cache::DependencyCache, symbols::SymbolType};
use crate::languages::traits::LanguageSupport;

use super::definition::utils::set_start_position;
use super::definition::{external, local, project, workspace};
use super::diagnostics::collect_syntax_errors;
use super::hover;
use super::implementation;
use super::utils::find_identifier_at_position;

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
        &[r#"(interface_declaration) @interface"#]
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

        ; Interface declarations
        (interface_declaration
          (type_identifier) @interface_decl)

        ; Parameters
        (parameter
          (simple_identifier) @param_decl)
        (class_parameter
          (simple_identifier) @param_decl)

        ; Function parameter types
        (function_declaration
          (function_value_parameters
            (parameter
              (user_type (type_identifier) @type_name))))

        (function_declaration
          (function_value_parameters
            (parameter
              (user_type (type_identifier) @type_name))))

        ; Interface method parameter types  
        (interface_declaration
          (class_body
            (function_declaration
              (function_value_parameters
                (parameter
                  (user_type (type_identifier) @type_name))))))

        (interface_declaration
          (class_body  
            (function_declaration
              (function_value_parameters
                (parameter
                  (user_type (type_identifier) @type_name))))))

        ; Function return type
        (function_declaration
          (user_type (type_identifier) @type_name))

        ; Interface method return types
        (interface_declaration
          (class_body
            (function_declaration
              (user_type (type_identifier) @type_name))))

        ; INHERITANCE
        ; Super class/interface in class declaration (treat as generic type)
        (class_declaration
          (delegation_specifier
            (user_type (type_identifier) @type_name)))
              
        ; Super interface in interface declaration (treat as generic type)
        (interface_declaration
          (delegation_specifier
            (user_type (type_identifier) @type_name)))

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

            let result =
                self.find_definition_chain(tree, source, dependency_cache, uri, &identifier_node);
            if let Err(e) = &result {
                warn!(
                    "Kotlin: Failed to find definition for '{}': {:?}",
                    identifier_text, e
                );
            }
            result
        } else {
            warn!(
                "Kotlin: No identifier found at position {:?} in {}",
                position, uri
            );
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
        let query = Query::new(&tree_sitter_kotlin::language(), query_text).map_err(|e| {
            anyhow!(
                "Failed to create Kotlin symbol type detection query: {:?}",
                e
            )
        })?;

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
                        "interface_decl" => SymbolType::InterfaceDeclaration,
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

        debug!("result: {:#?}", result);

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

    fn find_method_with_signature<'a>(
        &self,
        tree: &'a Tree,
        source: &str,
        method_name: &str,
        call_signature: &crate::languages::common::method_resolution::CallSignature,
    ) -> Option<tree_sitter::Node<'a>> {
        // Convert the common CallSignature to Kotlin's CallSignature
        let kotlin_call_sig = crate::languages::kotlin::definition::method_resolution::CallSignature {
            parameter_count: call_signature.arg_count,
            parameter_types: call_signature.arg_types.iter()
                .map(|t| t.as_deref().unwrap_or("Any").to_string())
                .collect(),
        };
        
        crate::languages::kotlin::definition::method_resolution::find_method_with_signature(
            tree, source, method_name, &kotlin_call_sig
        )
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

    fn extract_instance_method_context(
        &self,
        usage_node: &Node,
        source: &str,
    ) -> Option<(String, String)> {
        // Kotlin-specific: handle navigation_expression instead of method_invocation
        
        // Find parent navigation_expression or call_expression
        let mut current = Some(*usage_node);
        let mut nav_expr = None;
        
        while let Some(node) = current {
            if node.kind() == "navigation_expression" {
                nav_expr = Some(node);
                break;
            }
            if node.kind() == "call_expression" {
                // Check if it has a navigation_expression child
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "navigation_expression" {
                            nav_expr = Some(child);
                            break;
                        }
                    }
                }
                break;
            }
            current = node.parent();
        }
        
        if let Some(nav_node) = nav_expr {
            if let (Some(object_node), Some(nav_suffix)) = (nav_node.child(0), nav_node.child(1)) {
                if nav_suffix.kind() == "navigation_suffix" {
                    // Find the simple_identifier child (not the '.' token)
                    let method_node = (0..nav_suffix.child_count())
                        .filter_map(|i| nav_suffix.child(i))
                        .find(|child| child.kind() == "simple_identifier");
                    
                    if let Some(method_node) = method_node {
                        let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                        let method_name = method_node.utf8_text(source.as_bytes()).ok()?.to_string();
                        
                        // Check if this is an instance call (variable starts with lowercase)
                        if variable_name.chars().next().map_or(false, |c| c.is_lowercase()) {
                            return Some((variable_name, method_name));
                        }
                    }
                }
            }
        }
        None
    }

    fn extract_static_method_context(
        &self,
        usage_node: &Node,
        source: &str,
    ) -> Option<(String, String)> {
        // Kotlin-specific: handle companion object calls and static imports
        
        // Find parent navigation_expression
        let mut current = Some(*usage_node);
        let mut nav_expr = None;
        
        while let Some(node) = current {
            if node.kind() == "navigation_expression" {
                nav_expr = Some(node);
                break;
            }
            if node.kind() == "call_expression" {
                // Check if it has a navigation_expression child
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "navigation_expression" {
                            nav_expr = Some(child);
                            break;
                        }
                    }
                }
                break;
            }
            current = node.parent();
        }
        
        if let Some(nav_node) = nav_expr {
            if let (Some(object_node), Some(nav_suffix)) = (nav_node.child(0), nav_node.child(1)) {
                if nav_suffix.kind() == "navigation_suffix" {
                    // Find the simple_identifier child (not the '.' token)
                    let method_node = (0..nav_suffix.child_count())
                        .filter_map(|i| nav_suffix.child(i))
                        .find(|child| child.kind() == "simple_identifier");
                    
                    if let Some(method_node) = method_node {
                        let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                        let method_name = method_node.utf8_text(source.as_bytes()).ok()?.to_string();
                        
                        // Only return for static calls (class names start with uppercase)
                        if class_name.chars().next().map_or(false, |c| c.is_uppercase()) {
                            return Some((class_name, method_name));
                        }
                    }
                }
            }
        }
        None
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
        // For Kotlin static methods (companion object methods)
        // Find the class/interface definition first
        self.find_local(tree, source, file_uri, usage_node)
            .or_else(|| {
                self.find_in_project(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_in_workspace(source, file_uri, usage_node, dependency_cache.clone())
            })
            .or_else(|| {
                self.find_external(source, file_uri, usage_node, dependency_cache.clone())
            })
    }

    fn find_field_declaration_type(&self, field_name: &str, tree: &Tree, source: &str) -> Option<String> {
        self.extract_kotlin_variable_type(field_name, tree, source, &tree.root_node())
    }
    
    fn find_variable_declaration_type(&self, variable_name: &str, tree: &Tree, source: &str, usage_node: &Node) -> Option<String> {
        self.extract_kotlin_variable_type(variable_name, tree, source, usage_node)
    }
    
    fn find_parameter_type(&self, param_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        // TODO: Implement Kotlin parameter type extraction
        None
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
        
        // Step 1: Extract the variable type using Kotlin-specific logic
        let variable_type = self.extract_kotlin_variable_type(variable_name, tree, source, usage_node)?;
        
        // Step 2: Find the class/interface definition for this type
        if let Some(class_location) = self.find_kotlin_class_definition(&variable_type, tree, source, file_uri, dependency_cache.clone()) {
            // Step 3: Search for the specific method within the class/interface file
            use crate::languages::common::method_resolution::search_method_in_class_file_cross_language;
            if let Some(method_location) = search_method_in_class_file_cross_language(&class_location, method_name) {
                return Some(method_location);
            }
            
            return Some(class_location);
        }
        None
    }
}

impl KotlinSupport {
    /// Extract variable type using Kotlin-specific AST patterns
    fn extract_kotlin_variable_type(&self, variable_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        
        let query_text = r#"
            (class_parameter
              (simple_identifier) @param_name
              (user_type (type_identifier) @param_type))
        "#;
        
        if let Ok(query) = Query::new(&tree_sitter_kotlin::language(), query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                let mut found_name = None;
                let mut found_type = None;
                
                for capture in query_match.captures {
                    let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    let capture_name = query.capture_names()[capture.index as usize];
                    
                    if capture_name == "param_name" && capture_text == variable_name {
                        found_name = Some(capture_text);
                    } else if capture_name == "param_type" {
                        found_type = Some(capture_text.to_string());
                    }
                }
                
                if found_name.is_some() && found_type.is_some() {
                    let var_type = found_type.unwrap();
                    return Some(var_type);
                }
            }
        }
        None
    }
    
    /// Find class/interface definition for a given type name
    fn find_kotlin_class_definition(&self, type_name: &str, tree: &Tree, source: &str, file_uri: &str, dependency_cache: Arc<DependencyCache>) -> Option<Location> {
        
        // First try to find it locally in the current file
        if let Some(local_location) = self.find_local_class_definition(type_name, tree, source, file_uri) {
            return Some(local_location);
        }
        
        // Then search in project/workspace/external dependencies
        // Create a dummy usage node to search for the type name
        if let Some(type_node) = self.find_type_identifier_in_tree(type_name, tree, source) {
            return self.find_in_project(source, file_uri, &type_node, dependency_cache.clone())
                .or_else(|| self.find_in_workspace(source, file_uri, &type_node, dependency_cache.clone()))
                .or_else(|| self.find_external(source, file_uri, &type_node, dependency_cache.clone()));
        }
        
        None
    }
    
    /// Find a class/interface definition locally in the current file
    fn find_local_class_definition(&self, type_name: &str, tree: &Tree, source: &str, file_uri: &str) -> Option<Location> {
        let query_text = r#"
            (class_declaration (type_identifier) @class_name)
            (interface_declaration (type_identifier) @interface_name)
        "#;
        
        if let Ok(query) = Query::new(&tree_sitter_kotlin::language(), query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if capture_text == type_name {
                        let range = capture.node.range();
                        return Some(Location {
                            uri: tower_lsp::lsp_types::Url::from_file_path(file_uri).ok()?,
                            range: tower_lsp::lsp_types::Range {
                                start: Position {
                                    line: range.start_point.row as u32,
                                    character: range.start_point.column as u32,
                                },
                                end: Position {
                                    line: range.end_point.row as u32,
                                    character: range.end_point.column as u32,
                                },
                            },
                        });
                    }
                }
            }
        }
        
        None
    }
    
    /// Find a type identifier node in the tree for searching
    fn find_type_identifier_in_tree<'a>(&self, type_name: &str, tree: &'a Tree, source: &str) -> Option<Node<'a>> {
        let query_text = r#"(type_identifier) @type"#;
        
        if let Ok(query) = Query::new(&tree_sitter_kotlin::language(), query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    let capture_text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
                    if capture_text == type_name {
                        return Some(capture.node);
                    }
                }
            }
        }
        
        None
    }
}

impl Default for KotlinSupport {
    fn default() -> Self {
        Self::new()
    }
}
