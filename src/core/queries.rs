/// Trait for providing language-specific tree-sitter queries
pub trait QueryProvider {
    /// Queries for finding method declarations
    fn function_declaration_queries(&self) -> &[&'static str];

    /// Query for symbol type detection (single large query with multiple captures)
    fn symbol_type_detection_query(&self) -> &'static str;

    /// Queries for finding import statements
    fn import_queries(&self) -> &[&'static str];
}
