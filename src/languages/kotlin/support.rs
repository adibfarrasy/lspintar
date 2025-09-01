use std::sync::Arc;

use anyhow::{anyhow, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::warn;
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

    /// Check if a navigation expression node is actually accessing an enum constant
    /// This is a heuristic approach that checks common enum naming patterns
    fn is_enum_constant_access(&self, field_node: &Node, source: &str, tree: &Tree) -> bool {
        // For Kotlin, enum access is through navigation_expression
        // Find the parent navigation_expression node
        let navigation_expr = field_node.parent()
            .and_then(|p| if p.kind() == "navigation_suffix" { p.parent() } else { None })
            .and_then(|p| if p.kind() == "navigation_expression" { Some(p) } else { None });
        
        if let Some(nav_expr) = navigation_expr {
            // Get the first child which should be the object identifier
            if let Some(object_node) = nav_expr.child(0) {
                if let Ok(object_name) = object_node.utf8_text(source.as_bytes()) {
                    // First check if this object name refers to an enum in the same file
                    if self.is_enum_type_in_tree(object_name, tree, source) {
                        return true;
                    }
                    
                    // Heuristic: if the object name ends with "Enum" or contains "Enum",
                    // and the field name is ALL_CAPS, it's likely an enum constant
                    if let Ok(field_name) = field_node.utf8_text(source.as_bytes()) {
                        let looks_like_enum_type = object_name.contains("Enum") || 
                                                 object_name.ends_with("Status") ||
                                                 object_name.ends_with("Type") ||
                                                 object_name.ends_with("Mode") ||
                                                 object_name.ends_with("State");
                        
                        let looks_like_enum_constant = field_name.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit());
                        
                        if looks_like_enum_type && looks_like_enum_constant {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a given type name is an enum declaration in the current tree
    fn is_enum_type_in_tree(&self, type_name: &str, tree: &Tree, source: &str) -> bool {
        // In Kotlin, enums are class declarations with enum_class_body
        let query_text = r#"(class_declaration (type_identifier) @enum_name (enum_class_body))"#;
        
        if let Ok(query) = Query::new(&tree_sitter_kotlin::language(), query_text) {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
            
            while let Some(query_match) = matches.next() {
                for capture in query_match.captures {
                    if let Ok(enum_name) = capture.node.utf8_text(source.as_bytes()) {
                        if enum_name == type_name {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

impl QueryProvider for KotlinSupport {

    fn method_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(function_declaration) @function"#,
            r#"(primary_constructor) @constructor"#,
            r#"(secondary_constructor) @constructor"#,
        ]
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
                        "field_usage" => {
                            // Check if this is an enum constant access (e.g., SomeEnum.CONSTANT)
                            if self.is_enum_constant_access(node, source, tree) {
                                SymbolType::EnumUsage
                            } else {
                                SymbolType::FieldUsage
                            }
                        },
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
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(workspace::find_in_workspace(
                source, file_uri, usage_node, dependency_cache, self,
            ))
        })
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
        call_signature: &crate::languages::common::definition_chain::CallSignature,
    ) -> Option<tree_sitter::Node<'a>> {
        // Convert the common CallSignature to Kotlin's CallSignature
        let kotlin_call_sig = crate::languages::kotlin::definition::definition_chain::CallSignature {
            parameter_count: call_signature.arg_count,
        };
        
        crate::languages::kotlin::definition::definition_chain::find_method_with_signature(
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
        crate::languages::common::definition_chain::find_definition_chain(
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
                        
                        // Get the text of the node the user actually clicked on
                        let usage_text = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
                        
                        // Check if this is an instance call (variable starts with lowercase)
                        if variable_name.chars().next().map_or(false, |c| c.is_lowercase()) {
                            // Only return context if user navigated on the method name
                            if usage_text == method_name {
                                return Some((variable_name, method_name));
                            } else if usage_text == variable_name {
                                // User navigated on variable name - return None so go-to-definition goes to variable
                                return None;
                            }
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
        // Find parent call_expression with navigation_expression
        let mut current = usage_node.parent();
        
        while let Some(node) = current {
            if node.kind() == "call_expression" {
                // Check if this call has a navigation_expression child
                for i in 0..node.child_count() {
                    if let Some(nav_expr) = node.child(i) {
                        if nav_expr.kind() == "navigation_expression" {
                            // Extract class and method from navigation_expression
                            if let (Some(object_node), Some(nav_suffix)) = (nav_expr.child(0), nav_expr.child(1)) {
                                if nav_suffix.kind() == "navigation_suffix" {
                                    // Find the simple_identifier in nav_suffix
                                    let method_node = (0..nav_suffix.child_count())
                                        .filter_map(|i| nav_suffix.child(i))
                                        .find(|child| child.kind() == "simple_identifier");
                                    
                                    if let Some(method_node) = method_node {
                                        let class_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                                        let method_name = method_node.utf8_text(source.as_bytes()).ok()?.to_string();
                                        let usage_text = usage_node.utf8_text(source.as_bytes()).ok()?.to_string();
                                        
                                        // Only handle static calls (class names start with uppercase)
                                        if class_name.chars().next().map_or(false, |c| c.is_uppercase()) {
                                            if usage_text == method_name {
                                                return Some((class_name, method_name));
                                            } else if usage_text == class_name {
                                                return None; // Go to class definition instead
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                break; // Found call_expression, stop looking
            }
            current = node.parent();
        }
        None
    }

    fn extract_instance_field_context(
        &self,
        usage_node: &Node,
        source: &str,
    ) -> Option<(String, String)> {
        let usage_text = usage_node.utf8_text(source.as_bytes()).unwrap_or("");
        
        // Find parent navigation_expression or directly_assignable_expression
        let mut current = usage_node.parent();
        
        while let Some(node) = current {
            if node.kind() == "navigation_expression" {
                // Extract object and field from navigation_expression
                if let (Some(object_node), Some(nav_suffix)) = (node.child(0), node.child(1)) {
                    if nav_suffix.kind() == "navigation_suffix" {
                        // Find the simple_identifier in nav_suffix
                        let field_node = (0..nav_suffix.child_count())
                            .filter_map(|i| nav_suffix.child(i))
                            .find(|child| child.kind() == "simple_identifier");
                        
                        if let Some(field_node) = field_node {
                            let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                            let field_name = field_node.utf8_text(source.as_bytes()).ok()?.to_string();
                            
                            // Check if this is an instance field access (variable starts with lowercase)
                            if variable_name.chars().next().map_or(false, |c| c.is_lowercase()) {
                                // Only return context if user clicked on the field name
                                if usage_text == field_name {
                                    return Some((variable_name, field_name));
                                } else if usage_text == variable_name {
                                    // User clicked on variable name - return None so go-to-definition goes to variable
                                    return None;
                                }
                            }
                        }
                    }
                }
                break; // Found navigation_expression, stop looking
            }
            else if node.kind() == "directly_assignable_expression" {
                // For assignments like sms.body = "test", the structure is:
                // directly_assignable_expression -> simple_identifier (sms) + navigation_suffix -> simple_identifier (body)
                if node.child_count() >= 2 {
                    if let (Some(object_node), Some(nav_suffix)) = (node.child(0), node.child(1)) {
                        if object_node.kind() == "simple_identifier" && nav_suffix.kind() == "navigation_suffix" {
                            // Find the simple_identifier in nav_suffix
                            let field_node = (0..nav_suffix.child_count())
                                .filter_map(|i| nav_suffix.child(i))
                                .find(|child| child.kind() == "simple_identifier");
                            
                            if let Some(field_node) = field_node {
                                let variable_name = object_node.utf8_text(source.as_bytes()).ok()?.to_string();
                                let field_name = field_node.utf8_text(source.as_bytes()).ok()?.to_string();
                                
                                // Check if this is an instance field access (variable starts with lowercase)
                                if variable_name.chars().next().map_or(false, |c| c.is_lowercase()) {
                                    // Only return context if user clicked on the field name
                                    if usage_text == field_name {
                                        return Some((variable_name, field_name));
                                    } else if usage_text == variable_name {
                                        // User clicked on variable name - return None so go-to-definition goes to variable
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                }
                break; // Found directly_assignable_expression, stop looking
            }
            current = node.parent();
        }
        
        None
    }


    fn find_field_declaration_type(&self, field_name: &str, tree: &Tree, source: &str) -> Option<String> {
        self.extract_kotlin_variable_type(field_name, tree, source, &tree.root_node())
    }
    
    fn find_variable_declaration_type(&self, variable_name: &str, tree: &Tree, source: &str, usage_node: &Node) -> Option<String> {
        self.extract_kotlin_variable_type(variable_name, tree, source, usage_node)
    }
    
    fn find_parameter_type(&self, param_name: &str, tree: &Tree, source: &str, _usage_node: &Node) -> Option<String> {
        // Search for regular function parameters first
        let function_param_query = r#"
            (parameter
              (simple_identifier) @param_name
              (user_type (type_identifier) @param_type))
        "#;
        
        // Search for constructor parameters (which can be used directly as properties in Kotlin)
        let constructor_param_query = r#"
            (primary_constructor
              (class_parameter
                (simple_identifier) @param_name
                (user_type (type_identifier) @param_type)))
        "#;
        
        let language = tree_sitter_kotlin::language();
        
        // First, try function parameters
        if let Ok(query) = tree_sitter::Query::new(&language, function_param_query) {
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
        }
        
        // Then try constructor parameters
        if let Ok(query) = tree_sitter::Query::new(&language, constructor_param_query) {
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
        }
        
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
        // Use the common method resolution logic that handles signature matching and overloads
        crate::languages::common::definition_chain::find_instance_method_definition(
            self, tree, source, file_uri, usage_node, variable_name, method_name, dependency_cache
        )
    }

    fn extract_call_signature(&self, usage_node: &Node, source: &str) -> Option<crate::languages::common::definition_chain::CallSignature> {
        // Use Kotlin-specific signature extraction
        if let Some(kotlin_signature) = crate::languages::kotlin::definition::definition_chain::extract_call_signature_from_context(usage_node, source) {
            // Convert Kotlin's CallSignature to the common CallSignature format
            Some(crate::languages::common::definition_chain::CallSignature {
                arg_count: kotlin_signature.parameter_count,
                arg_types: vec![None; kotlin_signature.parameter_count], // Kotlin signature doesn't track types yet
            })
        } else {
            None
        }
    }

    fn resolve_type_fqn(&self, type_name: &str, source: &str, dependency_cache: &Arc<DependencyCache>) -> Option<String> {
        // Try to resolve through imports first
        if let Some(resolved_fqn) = super::definition::utils::resolve_symbol_with_imports(type_name, source, dependency_cache) {
            return Some(resolved_fqn);
        }
        
        // Fallback to current package + type name  
        if let Some(package) = super::definition::utils::extract_package_from_source(source) {
            if !package.is_empty() {
                Some(format!("{}.{}", package, type_name))
            } else {
                Some(type_name.to_string())
            }
        } else {
            Some(type_name.to_string())
        }
    }

    fn find_type_in_tree(&self, tree: &Tree, source: &str, type_name: &str, file_uri: &str) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};
        
        // Kotlin type queries covering classes, interfaces, objects, and enums
        let type_query_text = r#"
            (type_identifier) @type_name
        "#;
        let type_query = get_or_create_query(type_query_text).ok()?;
        
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&type_query, tree.root_node(), source.as_bytes());
        
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(captured_name) = capture.node.utf8_text(source.as_bytes()) {
                    if captured_name == type_name {
                        return crate::core::utils::node_to_lsp_location(&capture.node, file_uri);
                    }
                }
            }
        }
        
        None
    }

    fn find_method_in_tree(&self, tree: &Tree, source: &str, method_name: &str, file_uri: &str) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};
        
        let method_query_text = r#"
            (simple_identifier) @method_name
        "#;
        let method_query = get_or_create_query(method_query_text).ok()?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&method_query, tree.root_node(), source.as_bytes());

        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(captured_name) = capture.node.utf8_text(source.as_bytes()) {
                    if captured_name == method_name {
                        return crate::core::utils::node_to_lsp_location(&capture.node, file_uri);
                    }
                }
            }
        }

        None
    }

    fn find_property_in_tree(&self, tree: &Tree, source: &str, property_name: &str, file_uri: &str) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};

        let property_query_text = r#"
            (property_declaration (variable_declaration (simple_identifier) @property_name))
            (class_parameter (simple_identifier) @property_name)
        "#;
        let property_query = get_or_create_query(property_query_text).ok()?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&property_query, tree.root_node(), source.as_bytes());

        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(captured_name) = capture.node.utf8_text(source.as_bytes()) {
                    if captured_name == property_name {
                        return crate::core::utils::node_to_lsp_location(&capture.node, file_uri);
                    }
                }
            }
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
}

impl Default for KotlinSupport {
    fn default() -> Self {
        Self::new()
    }
}
