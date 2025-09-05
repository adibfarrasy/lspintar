use core::panic;
use std::sync::Arc;

use anyhow::{Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::constants::LSP_NAME;
use crate::core::queries::QueryProvider;
use crate::core::{dependency_cache::DependencyCache, symbols::SymbolType};
use crate::languages::groovy::definition::utils::get_wildcard_imports_from_source;
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

    /// Check if a field access node is actually accessing an enum constant
    /// This is a heuristic approach that checks common enum naming patterns
    #[tracing::instrument(skip_all)]
    fn is_enum_constant_access(&self, field_node: &Node, source: &str, tree: &Tree) -> bool {
        // Find the parent field_access node
        let field_access = field_node.parent().and_then(|p| {
            if p.kind() == "field_access" {
                Some(p)
            } else {
                None
            }
        });

        if let Some(field_access_node) = field_access {
            // Get the object part of the field access (e.g., "ResponseEnum" in "ResponseEnum.ILLEGAL_PRODUCTS")
            if let Some(object_node) = field_access_node.child_by_field_name("object") {
                if let Ok(object_name) = object_node.utf8_text(source.as_bytes()) {
                    // First check if this object name refers to an enum in the same file
                    if self.is_enum_type_in_tree(object_name, tree, source) {
                        return true;
                    }

                    // Heuristic: if the object name ends with "Enum" or contains "Enum",
                    // and the field name is ALL_CAPS, it's likely an enum constant
                    if let Ok(field_name) = field_node.utf8_text(source.as_bytes()) {
                        let looks_like_enum_type = object_name.contains("Enum")
                            || object_name.ends_with("Status")
                            || object_name.ends_with("Type")
                            || object_name.ends_with("Mode")
                            || object_name.ends_with("State");

                        let looks_like_enum_constant = field_name
                            .chars()
                            .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit());

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
    #[tracing::instrument(skip_all)]
    fn is_enum_type_in_tree(&self, type_name: &str, tree: &Tree, source: &str) -> bool {
        let query_text = r#"(enum_declaration name: (identifier) @enum_name)"#;

        if let Ok(query) = Query::new(&tree_sitter_groovy::language(), query_text) {
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

impl QueryProvider for GroovySupport {
    fn function_declaration_queries(&self) -> &[&'static str] {
        &[
            r#"(function_declaration) @method"#,
            r#"(constructor_declaration) @constructor"#,
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
        (function_declaration
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
        &[r#"(import_declaration) @import"#]
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
        (function_declaration
          name: (identifier) @method_decl)

        ; Enum declarations
        (enum_declaration
          name: (identifier) @enum_decl)

        ; Parameters
        (parameter
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
                                } else if capture_text
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
                            "type_name" => SymbolType::Type,
                            "super_interface" => SymbolType::SuperInterface,
                            "super_class" => SymbolType::SuperClass,
                            "field_usage" => {
                                // Check if this is an enum constant access (e.g., SomeEnum.CONSTANT)
                                if self.is_enum_constant_access(node, source, tree) {
                                    SymbolType::EnumUsage
                                } else {
                                    SymbolType::FieldUsage
                                }
                            }
                            "var_usage" => SymbolType::VariableUsage,
                            "potential_field_usage" => {
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
            tokio::runtime::Handle::current().block_on(find_in_project(
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
        recursion_depth: usize,
    ) -> Option<Location> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(find_in_workspace(
                source,
                file_uri,
                usage_node,
                dependency_cache,
                self,
                recursion_depth,
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
            tokio::runtime::Handle::current().block_on(find_external(
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
        crate::languages::common::definition_chain::find_definition_chain(
            self,
            tree,
            source,
            dependency_cache,
            file_uri,
            usage_node,
        )
    }

    fn find_instance_method_definition(
        &self,
        tree: &Tree,
        source: &str,
        file_uri: &str,
        usage_node: &Node,
        variable_name: &str,
        _method_name: &str,
        dependency_cache: Arc<DependencyCache>,
    ) -> Option<Location> {
        // Try to resolve the variable type using Groovy's type resolution methods
        let variable_type = self
            .find_field_declaration_type(variable_name, tree, source)
            .or_else(|| {
                self.find_variable_declaration_type(variable_name, tree, source, usage_node)
            })
            .or_else(|| self.find_parameter_type(variable_name, tree, source, usage_node));

        if let Some(_var_type) = variable_type {
            // Use the common method resolution to find the method in the type's class
            if let Some(location) =
                crate::languages::common::definition_chain::find_instance_method_definition(
                    self,
                    tree,
                    source,
                    file_uri,
                    usage_node,
                    variable_name,
                    _method_name,
                    dependency_cache,
                )
            {
                return Some(location);
            }
        }

        None
    }

    fn find_method_with_signature<'a>(
        &self,
        tree: &'a Tree,
        source: &str,
        _method_name: &str,
        call_signature: &crate::languages::common::definition_chain::CallSignature,
    ) -> Option<tree_sitter::Node<'a>> {
        let result =
            crate::languages::groovy::definition::definition_chain::find_method_with_signature(
                tree,
                source,
                _method_name,
                call_signature,
            );
        result
    }

    fn find_field_declaration_type(
        &self,
        field_name: &str,
        tree: &Tree,
        source: &str,
    ) -> Option<String> {
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
            Err(_) => {
                return None;
            }
        };

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        while let Some(query_match) = matches.next() {
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

    fn find_variable_declaration_type(
        &self,
        variable_name: &str,
        tree: &Tree,
        source: &str,
        _usage_node: &Node,
    ) -> Option<String> {
        let query_text = r#"
            (variable_declaration 
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

    fn find_parameter_type(
        &self,
        param_name: &str,
        tree: &Tree,
        source: &str,
        _usage_node: &Node,
    ) -> Option<String> {
        let query_text = r#"
            (parameter
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

    fn resolve_type_fqn(
        &self,
        type_name: &str,
        source: &str,
        dependency_cache: &Arc<DependencyCache>,
    ) -> Option<String> {
        // Try to resolve through imports first
        if let Some(resolved_fqn) = super::definition::utils::resolve_symbol_with_imports(
            type_name,
            source,
            dependency_cache,
        ) {
            return Some(resolved_fqn);
        }

        // Fallback to current package + type name
        if let Some(package) = super::definition::project::extract_package_from_source(source) {
            if !package.is_empty() {
                Some(format!("{}.{}", package, type_name))
            } else {
                Some(type_name.to_string())
            }
        } else {
            Some(type_name.to_string())
        }
    }

    fn find_type_in_tree(
        &self,
        tree: &Tree,
        source: &str,
        type_name: &str,
        file_uri: &str,
    ) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};

        // Groovy type queries covering classes, interfaces, enums, and annotation types
        let type_query_text = r#"
            (class_declaration name: (identifier) @type_name)
            (interface_declaration name: (identifier) @type_name)
            (enum_declaration name: (identifier) @type_name)
            (annotation_type_declaration name: (identifier) @type_name)
        "#;
        let type_query = get_or_create_query(type_query_text, &tree_sitter_groovy::language())?;

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

    fn find_method_in_tree(
        &self,
        tree: &Tree,
        source: &str,
        method_name: &str,
        file_uri: &str,
    ) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};

        let method_query_text = r#"
            (function_declaration name: (identifier) @method_name)
            (constructor_declaration name: (identifier) @method_name)
        "#;
        let method_query = get_or_create_query(method_query_text, &tree_sitter_groovy::language())?;

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

    fn find_property_in_tree(
        &self,
        tree: &Tree,
        source: &str,
        property_name: &str,
        file_uri: &str,
    ) -> Option<Location> {
        use super::definition::utils::get_or_create_query;
        use tree_sitter::{QueryCursor, StreamingIterator};

        let property_query_text = r#"
            (field_declaration declarator: (variable_declarator name: (identifier) @property_name))
        "#;
        let property_query =
            get_or_create_query(property_query_text, &tree_sitter_groovy::language())?;

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

impl GroovySupport {
    /// Check if an identifier is an imported class name
    #[tracing::instrument(skip_all)]
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
    #[tracing::instrument(skip_all)]
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
        if parser
            .set_language(&tree_sitter_groovy::language())
            .is_err()
        {
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

                    if import_path.ends_with(&format!(".{}", class_name))
                        || import_path == class_name
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Check if there's a wildcard import that could include this class
    #[tracing::instrument(skip_all)]
    fn has_wildcard_import_for_class(&self, class_name: &str, source: &str) -> bool {
        // For now, we'll be conservative and assume uppercase class names in wildcard imports are likely classes
        // This could be enhanced by checking against the symbol index
        if let Some(wildcard_packages) = get_wildcard_imports_from_source(source) {
            // If there are wildcard imports and this looks like a class name (uppercase), assume it could be imported
            return !wildcard_packages.is_empty()
                && class_name
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_uppercase());
        }
        false
    }
}

impl Default for GroovySupport {
    fn default() -> Self {
        Self::new()
    }
}
