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

use crate::Indexer;
use crate::constants::{get_cache_dir, get_cfr_jar_path};
use crate::lsp_convert::{AsLspHover, AsLspLocation};
use crate::models::symbol::SymbolMetadata;
use crate::models::util::build_hover_parts;

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct ExternalSymbol {
    pub id: Option<i64>,
    pub jar_path: String,
    pub alt_jar_path: Option<String>,
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
        let from_sources = self.needs_decompilation
            && cached_path.extension().and_then(|e| e.to_str()) != Some("class");
        let uri = Url::from_file_path(cached_path).ok()?;
        let range = if from_sources {
            // Precise location unknown from bytecode indexing; open at top of file
            Range::new(Position::new(0, 0), Position::new(0, 0))
        } else {
            Range::new(
                Position::new(self.ident_line_start as u32, self.ident_char_start as u32),
                Position::new(self.ident_line_end as u32, self.ident_char_end as u32),
            )
        };
        Some(Location { uri, range })
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

        // NOTE: prefer sources over decompilation if available
        if self.needs_decompilation {
            if let Some(alt_jar) = &self.alt_jar_path {
                if let Ok(file) = File::open(alt_jar) {
                    if let Ok(mut archive) = ZipArchive::new(file) {
                        let stem = PathBuf::from(&self.source_file_path).with_extension("");
                        let stem_str = stem.to_string_lossy();
                        let entry_name = (0..archive.len()).find_map(|i| {
                            let entry = archive.by_index(i).ok()?;
                            let name = entry.name().to_string();
                            let entry_stem = PathBuf::from(&name).with_extension("");
                            if entry_stem.to_string_lossy() == stem_str {
                                Some(name)
                            } else {
                                None
                            }
                        });
                        if let Some(entry_name) = entry_name {
                            if let Ok(mut entry) = archive.by_name(&entry_name) {
                                let src_outpath = extract_dir.join(&entry_name);
                                if let Some(p) = src_outpath.parent() {
                                    fs::create_dir_all(p)?;
                                }
                                let mut outfile = File::create(&src_outpath)?;
                                copy(&mut entry, &mut outfile)?;
                                return Ok(src_outpath);
                            }
                        }
                    }
                }
            }
        }

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

    pub async fn with_sources(&self, indexer: Option<&Indexer>) -> Self {
        let Some(indexer) = indexer else {
            return self.clone();
        };
        if !self.needs_decompilation {
            return self.clone();
        }
        let Some(alt_jar) = &self.alt_jar_path else {
            return self.clone();
        };
        let alt_jar = PathBuf::from(alt_jar);
        let fqn = self.fully_qualified_name.clone();
        let Ok(Ok((src_symbols, _))) = tokio::task::spawn_blocking({
            let indexer = indexer.clone();
            move || indexer.extract_jar_symbols(&alt_jar, None)
        })
        .await
        else {
            return self.clone();
        };
        for chunk in src_symbols.chunks(1000) {
            match indexer.repo.insert_external_symbols(chunk).await {
                Ok(_) => tracing::info!("inserted {} src symbols", chunk.len()),
                Err(e) => tracing::warn!("failed to insert src symbols: {e}"),
            }
        }
        let Some(src_sym) = src_symbols.iter().find(|s| s.fully_qualified_name == fqn) else {
            return self.clone();
        };
        let mut enriched = self.clone();
        enriched.jar_path = src_sym.jar_path.clone();
        enriched.source_file_path = src_sym.source_file_path.clone();
        enriched.needs_decompilation = false;
        enriched.ident_line_start = src_sym.ident_line_start;
        enriched.ident_line_end = src_sym.ident_line_end;
        enriched.ident_char_start = src_sym.ident_char_start;
        enriched.ident_char_end = src_sym.ident_char_end;
        enriched
    }
}
