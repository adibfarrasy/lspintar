use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write, copy};
use std::path::PathBuf;

use lsp_core::util::decompile_class;
use sqlx::{FromRow, types::Json};
use tower_lsp::lsp_types::{
    Hover, HoverContents, Location, MarkupContent, MarkupKind, Position, Range, Url,
};
use zip::ZipArchive;

use crate::constants::{get_cache_dir, get_cfr_jar_path};
use crate::lsp_convert::{AsLspHover, AsLspLocation};
use crate::models::symbol::SymbolMetadata;
use crate::models::util::build_hover_parts;

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
    pub file_type: String,
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
    pub needs_decompilation: bool,
    #[sqlx(json)]
    pub metadata: Json<SymbolMetadata>,
    pub last_modified: i64,
}

impl AsLspLocation for ExternalSymbol {
    fn as_lsp_location(&self) -> Option<Location> {
        let cached_path = self.extract_to_cache().ok()?;

        let uri = Url::from_file_path(cached_path).ok()?;

        Some(Location {
            uri,
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

impl AsLspHover for ExternalSymbol {
    fn as_lsp_hover(&self) -> Option<Hover> {
        let parts = build_hover_parts(
            &self.file_type,
            &self.package_name,
            &self.short_name,
            &self.symbol_type,
            &self.modifiers,
            &self.metadata,
        );
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: parts.join("\n"),
            }),
            range: None,
        })
    }
}

impl ExternalSymbol {
    pub fn extract_to_cache(&self) -> Result<PathBuf, Box<dyn Error>> {
        let mut hasher = DefaultHasher::new();
        self.jar_path.hash(&mut hasher);
        self.source_file_path.hash(&mut hasher);
        self.needs_decompilation.hash(&mut hasher);
        let jar_hash = hasher.finish();

        let extract_dir = get_cache_dir().join(jar_hash.to_string());

        let outpath = if self.needs_decompilation {
            extract_dir
                .join(&self.source_file_path)
                .with_extension("java")
        } else {
            extract_dir.join(&self.source_file_path)
        };

        if outpath.exists() {
            return Ok(outpath);
        }

        let file = File::open(&self.jar_path)?;
        let mut archive = ZipArchive::new(file)?;

        match archive.by_name(&self.source_file_path) {
            Ok(mut file) => {
                if let Some(p) = outpath.parent()
                    && !p.exists()
                {
                    fs::create_dir_all(p)?;
                }

                if self.needs_decompilation {
                    let mut buffer = Vec::new();
                    file.read_to_end(&mut buffer)?;
                    let class_name = self
                        .fully_qualified_name
                        .split_once('#')
                        .map(|(name, _)| name)
                        .unwrap_or(&self.fully_qualified_name);
                    let source_code = decompile_class(class_name, &buffer, &get_cfr_jar_path())?;

                    let mut outfile = File::create(&outpath)?;
                    outfile.write_all(source_code.as_bytes())?;
                } else {
                    let mut outfile = File::create(&outpath)?;
                    copy(&mut file, &mut outfile)?;
                }
            }
            Err(_) => {
                return Err(format!(
                    "File '{}' not found in JAR '{}'",
                    self.source_file_path, self.jar_path
                )
                .into());
            }
        }

        Ok(outpath)
    }
}
