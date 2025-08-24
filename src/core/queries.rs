/// Trait for providing language-specific tree-sitter queries
pub trait QueryProvider {
    /// Queries for finding variable declarations (including parameters, local vars, fields)
    fn variable_declaration_queries(&self) -> &[&'static str];

    /// Queries for finding method declarations
    fn method_declaration_queries(&self) -> &[&'static str];

    /// Queries for finding class declarations
    fn class_declaration_queries(&self) -> &[&'static str];

    /// Queries for finding interface declarations
    fn interface_declaration_queries(&self) -> &[&'static str];

    /// Queries for finding parameter declarations
    fn parameter_queries(&self) -> &[&'static str];

    /// Queries for finding field declarations
    fn field_declaration_queries(&self) -> &[&'static str];

    /// Query for symbol type detection (single large query with multiple captures)
    fn symbol_type_detection_query(&self) -> &'static str;

    /// Queries for finding import statements
    fn import_queries(&self) -> &[&'static str];

    /// Queries for finding package declarations
    fn package_queries(&self) -> &[&'static str];
}
