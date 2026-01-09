use serde::{Deserialize, Serialize};
use sqlx::{FromRow, types::Json};

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct Symbol {
    pub id: Option<i64>,
    pub vcs_branch: String,
    pub short_name: String,
    pub fully_qualified_name: String,
    pub parent_name: Option<String>,
    pub file_path: String,
    pub file_type: String,
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

    pub extends_name: Option<String>,

    #[sqlx(json)]
    pub implements_names: Json<Vec<String>>,

    #[sqlx(json)]
    pub metadata: Json<SymbolMetadata>,
    pub last_modified: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolParameter {
    pub name: String,
    pub type_name: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<SymbolParameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<String>>,
}
