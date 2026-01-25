use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use lsp_core::util::extract_jar_to_cache;
use sqlx::{FromRow, types::Json};
use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::as_lsp_location::AsLspLocation;
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

impl AsLspLocation for ExternalSymbol {
    fn as_lsp_location(&self) -> Option<Location> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lspintar/sources");

        // Hash jar_path
        let mut hasher = DefaultHasher::new();
        self.jar_path.hash(&mut hasher);
        let jar_hash = hasher.finish();

        let extract_dir = cache_dir.join(jar_hash.to_string());

        if !extract_dir.exists() {
            extract_jar_to_cache(&self.jar_path, cache_dir).ok()?;
        }

        let full_path = extract_dir.join(&self.source_file_path);
        let uri = Url::from_file_path(full_path).ok()?;

        Some(Location {
            uri: uri,
            range: Range {
                start: Position {
                    line: self.ident_line_start as u32,
                    character: self.ident_char_start as u32,
                },
                end: Position {
                    line: self.ident_line_end as u32,
                    character: self.ident_char_end as u32,
                },
            },
        })
    }
}
