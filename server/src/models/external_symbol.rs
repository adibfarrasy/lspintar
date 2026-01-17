use sqlx::{FromRow, types::Json};

use crate::models::symbol::SymbolMetadata;

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct ExternalSymbol {
    pub id: Option<i64>,
    pub jar_path: String,
    pub source_file_path: String,
    pub short_name: String,
    pub fully_qualified_name: String,
    pub package_name: String,
    pub parent_name: Option<String>,
    pub symbol_type: String,
    #[sqlx(json)]
    pub modifiers: Json<Vec<String>>,
    pub line_start: i64,
    pub line_end: i64,
    pub char_start: i64,
    pub char_end: i64,
    pub ident_line_start: i64,
    pub ident_line_end: i64,
    pub ident_char_start: i64,
    pub ident_char_end: i64,
    pub is_decompiled: bool,
    #[sqlx(json)]
    pub metadata: Json<SymbolMetadata>,
    pub last_modified: i64,
}
