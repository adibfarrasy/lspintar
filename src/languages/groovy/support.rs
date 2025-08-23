use core::panic;
use std::sync::Arc;

use anyhow::{Context, Result};
use tower_lsp::lsp_types::{Diagnostic, Hover, Location, Position};
use tracing::debug;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::constants::LSP_NAME;
use crate::core::{
    dependency_cache::DependencyCache,
    symbols::SymbolType,
    utils::{uri_to_path, find_project_root, path_to_file_uri},
    definition::queries::QueryProvider,
};
use crate::languages::groovy::definition::method_resolution::extract_call_signature_from_context;
use crate::languages::groovy::definition::utils::{extract_instance_method_context, extract_static_method_context, get_wildcard_imports_from_source, prepare_symbol_lookup_key_with_wildcard_support, resolve_variable_type, search_definition_in_project, search_static_method_definition_in_project};
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
        debug!("determine_symbol_type_from_context: analyzing symbol '{}' at position {:?}", node_text, node.range());

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
                        debug!("determine_symbol_type_from_context: MATCHED capture '{}' for symbol '{}'", capture_name, node_text);
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
                                debug!("method_object '{}' found", capture_text);
                                // Check if this is an imported class first (most reliable)
                                if self.is_imported_class(capture_text, source) {
                                    debug!("method_object '{}' is an imported class", capture_text);
                                    SymbolType::Type
                                } else if capture_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                                    // Uppercase first letter suggests class name (static method call)
                                    debug!("method_object '{}' looks like a class name (uppercase)", capture_text);
                                    SymbolType::Type
                                } else {
                                    // Lowercase first letter suggests variable (instance method call)
                                    debug!("method_object '{}' looks like a variable (lowercase)", capture_text);
                                    SymbolType::VariableUsage
                                }
                            }
                            "method_name" => {
                                debug!("method_name '{}' found", capture_text);
                                SymbolType::MethodCall
                            }
                            "simple_method_name" => {
                                debug!("simple_method_name '{}' found", capture_text);
                                SymbolType::MethodCall
                            }
                            "method_usage" => SymbolType::MethodCall,
                            "type_name" => SymbolType::Type,
                            "super_interface" => SymbolType::SuperInterface,
                            "super_class" => SymbolType::SuperClass,
                            "field_usage" => SymbolType::FieldUsage,
                            "var_usage" => SymbolType::VariableUsage,
                            "potential_field_usage" => {
                                debug!("potential_field_usage '{}' found", capture_text);
                                // Check case to distinguish types from variables
                                if capture_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                                    debug!("potential_field_usage '{}' looks like a type (uppercase)", capture_text);
                                    SymbolType::Type
                                } else {
                                    debug!("potential_field_usage '{}' looks like a variable (lowercase)", capture_text);
                                    SymbolType::VariableUsage
                                }
                            },
                            
                            "debug_method_invocation" => {
                                debug!("Found method_invocation at range {:?}", capture.node.range());
                                SymbolType::VariableUsage // Don't actually use this, just for debugging
                            },

                            _ => SymbolType::VariableUsage,
                        };

                        debug!("determine_symbol_type_from_context: final symbol type for '{}': {:?}", node_text, symbol);
                        result = Ok(symbol);
                        found = true;
                    }
                }
            });

        if !any_captures_found {
            debug!("determine_symbol_type_from_context: NO CAPTURES FOUND for '{}' - query may be invalid", node_text);
        } else if !found {
            debug!("determine_symbol_type_from_context: captures found but none matched '{}' at {:?}", node_text, node.range());
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
        // Check if this is a static method call pattern FIRST - before symbol type determination
        if let Some((class_name, method_name)) = extract_static_method_context(usage_node, source) {
            // For static method calls, we need to resolve the class first, then find the method
            if let Some(location) = self.find_static_method_definition(tree, source, file_uri, usage_node, &class_name, &method_name, dependency_cache.clone()) {
                return Ok(location);
            }
        }

        // Check if this is an instance method call pattern (do this EARLY, before fast-path)
        let instance_context = extract_instance_method_context(usage_node, source);
        if let Some((variable_name, method_name)) = instance_context {
            // For instance method calls, we need to resolve the variable type first, then find the method
            if let Some(location) = self.find_instance_method_definition(tree, source, file_uri, usage_node, &variable_name, &method_name, dependency_cache.clone()) {
                return Ok(location);
            }
        }
        
        // Optimized: Fast-path for common local cases
        let symbol_type = self.determine_symbol_type_from_context(tree, usage_node, source).ok();
        
        // Handle MethodCall separately - needs instance method detection, not local resolution
        if let Some(SymbolType::MethodCall) = symbol_type {
            // Skip fast-path for method calls
        } else if let Some(symbol_type) = symbol_type {
            if matches!(symbol_type, 
                SymbolType::VariableUsage | 
                SymbolType::ParameterDeclaration |
                SymbolType::FieldUsage
            ) {
                if let Some(local_location) = self.find_local(tree, source, file_uri, usage_node) {
                    return Ok(local_location);
                }
                
                // For local method calls that aren't found locally, 
                // they're likely in the same project - skip expensive workspace/external search
                if symbol_type == SymbolType::MethodCall {
                    if let Some(project_location) = self.find_in_project(source, file_uri, usage_node, dependency_cache.clone()) {
                        // If the definition is in the same file, don't call set_start_position 
                        // as it may find the wrong identifier with the same name
                        if project_location.uri.to_string() == file_uri {
                            return Ok(project_location);
                        } else {
                            let uri_string = project_location.uri.to_string();
                            // Skip set_start_position for builtin sources as they are already correctly positioned
                            if uri_string.contains("lspintar_builtin_sources") {
                                return Ok(project_location);
                            } else {
                                if let Some(final_location) = self.set_start_position(source, usage_node, &uri_string) {
                                    return Ok(final_location);
                                }
                            }
                        }
                    }
                }
            }
        }


        // Try local resolution again if not in fast-path
        if let Some(local_location) = self.find_local(tree, source, file_uri, usage_node) {
            return Ok(local_location);
        }
        
        // Sequential cross-file resolution to avoid race conditions
        self.find_cross_file_sequential(source, file_uri, usage_node, dependency_cache)
            .ok_or_else(|| anyhow::anyhow!("Definition not found"))
    }


}

impl GroovySupport {
    /// Check if an identifier is an imported class name
    fn is_imported_class(&self, class_name: &str, source: &str) -> bool {
        // Check for specific import
        if self.has_specific_import(class_name, source) {
            return true;
        }
        
        // Check for wildcard import that could include this class
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
                    // Extract just the import path from "import com.example.package.ClassName"
                    let import_path = import_text
                        .trim_start_matches("import")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    
                    // Check if it ends with our class name
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
        debug!("find_static_method_definition: resolving static method {}.{}", class_name, method_name);
        
        // Create a temporary node representing the class name to resolve the class first
        let class_node = self.create_temporary_class_node(tree, source, class_name);
        if class_node.is_none() {
            debug!("find_static_method_definition: failed to create temporary class node for '{}'", class_name);
            return None;
        }
        let class_node = class_node?;
        
        // Extract call signature for method matching
        let call_signature = extract_call_signature_from_context(usage_node, source);
        
        // Try to resolve the class through import resolution first, then normal resolution chain
        debug!("find_static_method_definition: trying to resolve class '{}'", class_name);
        
        // First try to resolve through imports using the enhanced lookup
        let class_location = if let Some((project_root, fqn)) = prepare_symbol_lookup_key_with_wildcard_support(
            &class_node, source, file_uri, None, &dependency_cache
        ) {
            debug!("find_static_method_definition: found import resolution for class '{}' -> '{}'", class_name, fqn);
            
            // Debug: show what's in the symbol index
            debug!("find_static_method_definition: symbol index contains {} entries", dependency_cache.symbol_index.len());
            let matching_entries: Vec<_> = dependency_cache.symbol_index.iter()
                .filter(|entry| entry.key().1.contains(&class_name))
                .take(5) // Show max 5 entries
                .map(|entry| format!("({:?}, {})", entry.key().0.file_name().unwrap_or_default(), entry.key().1))
                .collect();
            debug!("find_static_method_definition: entries containing '{}': {:?}", class_name, matching_entries);
            
            // Look up the class in the symbol index
            let symbol_key = (project_root.clone(), fqn.clone());
            debug!("find_static_method_definition: looking for symbol_key: ({:?}, {})", symbol_key.0.file_name().unwrap_or_default(), symbol_key.1);
            
            if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                debug!("find_static_method_definition: found class file at {:?}", file_location);
                let class_uri = path_to_file_uri(&file_location)?;
                Some(tower_lsp::lsp_types::Location {
                    uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                    range: tower_lsp::lsp_types::Range::default(),
                })
            } else {
                debug!("find_static_method_definition: class '{}' not found in symbol index for FQN '{}'", class_name, fqn);
                
                // Try looking in other project roots
                let workspace_projects: Vec<std::path::PathBuf> = dependency_cache
                    .symbol_index
                    .iter()
                    .map(|entry| entry.key().0.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                    
                debug!("find_static_method_definition: trying {} other workspace projects", workspace_projects.len());
                for other_project in workspace_projects {
                    if other_project == project_root {
                        continue;
                    }
                    
                    let symbol_key = (other_project.clone(), fqn.clone());
                    if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                        debug!("find_static_method_definition: found class in other project {:?}", other_project.file_name().unwrap_or_default());
                        let class_uri = path_to_file_uri(&file_location)?;
                        return Some(tower_lsp::lsp_types::Location {
                            uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                            range: tower_lsp::lsp_types::Range::default(),
                        });
                    }
                }
                debug!("find_static_method_definition: class not found in any project");
                None
            }
        } else {
            debug!("find_static_method_definition: no import resolution found for class '{}'", class_name);
            None
        }.or_else(|| {
            // Fallback to normal resolution chain
            debug!("find_static_method_definition: trying normal resolution chain for class '{}'", class_name);
            self.find_local(tree, source, file_uri, &class_node)
                .or_else(|| {
                    debug!("find_static_method_definition: local resolution failed, trying project");
                    self.find_in_project(source, file_uri, &class_node, dependency_cache.clone())
                })
                .or_else(|| {
                    debug!("find_static_method_definition: project resolution failed, trying workspace");
                    self.find_in_workspace(source, file_uri, &class_node, dependency_cache.clone())
                })
                .or_else(|| {
                    debug!("find_static_method_definition: workspace resolution failed, trying external");
                    self.find_external(source, file_uri, &class_node, dependency_cache.clone())
                })
        });
            
        if class_location.is_none() {
            debug!("find_static_method_definition: failed to resolve class '{}' in any scope", class_name);
            return None;
        }
        let class_location = class_location?;
        debug!("find_static_method_definition: successfully resolved class '{}' to location '{}'", class_name, class_location.uri);

        // Now search for the method in the resolved class file
        let method_location = if let Some(call_sig) = call_signature {
            debug!("find_static_method_definition: searching for method '{}' with signature in '{}'", method_name, class_location.uri);
            search_static_method_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        } else {
            debug!("find_static_method_definition: searching for method '{}' without signature in '{}'", method_name, class_location.uri);
            // Fallback to regular method search
            search_definition_in_project(
                file_uri,
                source,
                usage_node,
                &class_location.uri.to_string(),
                self
            )
        };
        
        if method_location.is_some() {
            debug!("find_static_method_definition: successfully found method '{}'", method_name);
        } else {
            debug!("find_static_method_definition: failed to find method '{}' in class file", method_name);
        }
        
        method_location
    }

    /// Create a temporary node for class name resolution
    fn create_temporary_class_node<'a>(&self, tree: &'a Tree, source: &str, class_name: &str) -> Option<Node<'a>> {
        // This is a workaround - we need to find an actual node in the tree that represents the class name
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
        debug!("find_instance_method_definition: resolving instance method {}.{}", variable_name, method_name);
        
        // First, resolve the variable to find its type
        debug!("find_instance_method_definition: resolving variable type for '{}'", variable_name);
        let variable_type = resolve_variable_type(variable_name, tree, source, usage_node);
        if variable_type.is_none() {
            debug!("find_instance_method_definition: failed to resolve variable type for '{}'", variable_name);
            return None;
        }
        let variable_type = variable_type.unwrap();
        debug!("find_instance_method_definition: resolved variable '{}' to type '{}'", variable_name, variable_type);
        
        // Extract call signature for method matching
        let call_signature = extract_call_signature_from_context(usage_node, source);
        debug!("find_instance_method_definition: extracted call signature: {:?}", call_signature.is_some());
        
        // Create a simple temporary node for the class name and use existing resolution chain
        debug!("find_instance_method_definition: resolving class location for type '{}'", variable_type);
        let class_location = self.resolve_class_through_standard_chain(&variable_type, tree, source, file_uri, dependency_cache.clone());

        if class_location.is_none() {
            debug!("find_instance_method_definition: failed to resolve class location for type '{}'", variable_type);
            return None;
        }
        let class_location = class_location.unwrap();
        debug!("find_instance_method_definition: resolved class '{}' to location '{}'", variable_type, class_location.uri);

        // Now search for the method in the resolved class file
        debug!("find_instance_method_definition: searching for method '{}' in class file '{}'", method_name, class_location.uri);
        
        // Try regular method search first (more permissive)
        debug!("find_instance_method_definition: trying regular method search first");
        let result = search_definition_in_project(
            file_uri,
            source,
            usage_node,
            &class_location.uri.to_string(),
            self
        );
        
        // If that fails and we have a signature, try signature-based search as fallback
        let result = if result.is_none() && call_signature.is_some() {
            debug!("find_instance_method_definition: regular search failed, trying signature-based method search");
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

        if result.is_some() {
            debug!("find_instance_method_definition: successfully found method '{}'", method_name);
        } else {
            debug!("find_instance_method_definition: failed to find method '{}' in class file", method_name);
        }

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
        debug!("resolve_class_through_standard_chain: resolving class '{}'", class_name);
        
        // Use the existing utility functions to resolve the class name
        // This leverages all the existing import resolution, wildcard resolution, etc.
        let current_file_path = uri_to_path(file_uri)?;
        let project_root = find_project_root(&current_file_path)?;
        debug!("resolve_class_through_standard_chain: project_root = {:?}", project_root.file_name().unwrap_or_default());
        
        // Direct lookup approach - try different FQN possibilities for the class name
        debug!("resolve_class_through_standard_chain: trying direct lookup for class '{}'", class_name);
        
        // Strategy 1: Try direct class name lookup
        let direct_symbol_key = (project_root.clone(), class_name.to_string());
        debug!("resolve_class_through_standard_chain: trying direct lookup with key ({:?}, {})", direct_symbol_key.0.file_name().unwrap_or_default(), direct_symbol_key.1);
        if let Some(file_location) = dependency_cache.symbol_index.get(&direct_symbol_key) {
            debug!("resolve_class_through_standard_chain: found class file at {:?} via direct lookup", file_location);
            let class_uri = path_to_file_uri(&file_location)?;
            return Some(Location {
                uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                range: tower_lsp::lsp_types::Range::default(),
            });
        }
        
        // Strategy 2: Try to find class name in any FQN in the symbol index
        debug!("resolve_class_through_standard_chain: searching all FQNs for class name '{}'", class_name);
        for entry in dependency_cache.symbol_index.iter() {
            let (entry_project_root, fqn) = entry.key();
            if fqn.ends_with(&format!(".{}", class_name)) || fqn == class_name {
                debug!("resolve_class_through_standard_chain: found matching FQN '{}' in project {:?}", fqn, entry_project_root.file_name().unwrap_or_default());
                let class_uri = path_to_file_uri(entry.value())?;
                return Some(Location {
                    uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                    range: tower_lsp::lsp_types::Range::default(),
                });
            }
        }
        
        // Strategy 3: Try the old method with identifier lookup as fallback
        debug!("resolve_class_through_standard_chain: falling back to identifier-based lookup for '{}'", class_name);
        if let Some(mock_node) = self.find_identifier_node_in_tree(tree, source, class_name) {
            debug!("resolve_class_through_standard_chain: found identifier node for '{}'", class_name);
            // Use the existing resolution utilities
            if let Some((_, fqn)) = prepare_symbol_lookup_key_with_wildcard_support(
                &mock_node, source, file_uri, Some(project_root.clone()), &dependency_cache
            ) {
                debug!("resolve_class_through_standard_chain: resolved '{}' to FQN '{}'", class_name, fqn);
                
                // Look up the class in the symbol index
                let symbol_key = (project_root.clone(), fqn.clone());
                debug!("resolve_class_through_standard_chain: looking up symbol_key ({:?}, {})", symbol_key.0.file_name().unwrap_or_default(), symbol_key.1);
                if let Some(file_location) = dependency_cache.symbol_index.get(&symbol_key) {
                    debug!("resolve_class_through_standard_chain: found class file at {:?}", file_location);
                    let class_uri = path_to_file_uri(&file_location)?;
                    return Some(Location {
                        uri: tower_lsp::lsp_types::Url::parse(&class_uri).ok()?,
                        range: tower_lsp::lsp_types::Range::default(),
                    });
                } else {
                    debug!("resolve_class_through_standard_chain: class '{}' not found in symbol index for FQN '{}'", class_name, fqn);
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
            } else {
                debug!("resolve_class_through_standard_chain: failed to resolve '{}' to FQN", class_name);
            }
        } else {
            debug!("resolve_class_through_standard_chain: no identifier node found for '{}'", class_name);
        }
        
        debug!("resolve_class_through_standard_chain: all strategies failed for class '{}'", class_name);
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
        // Strategy 1: Smart ordering - try most likely to succeed first
        let symbol_name = usage_node.utf8_text(source.as_bytes()).ok()?;
        
        // For simple symbols, try project first (most common case)
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
        
        // Strategy 2: Sequential I/O operations for workspace and external (non-parallel to avoid race conditions)
        // Try workspace first
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
        
        // Then try external
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
        
        // Strategy 3: Final fallback with project search if not done above
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
