use lsp_core::{
    language_support::LanguageSupport, node_types::NodeType, util::naive_resolve_fqn,
    vcs::VcsHandler,
};
use std::{collections::HashMap, fs::File, io::Read, path::Path, sync::Arc};
use zip::ZipArchive;

use crate::{
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata, SymbolParameter},
        symbol_super_mapping::SymbolSuperMapping,
    },
    repo::Repository,
};

use anyhow::{Context, Result, anyhow};
use sqlx::types::Json;
use tree_sitter::{Node, Tree};
use walkdir::WalkDir;

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Indexer {
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
    repo: Arc<Repository>,
    vcs: Arc<dyn VcsHandler>,
}

impl Indexer {
    pub fn new(repo: Arc<Repository>, vcs: Arc<dyn VcsHandler>) -> Self {
        Self {
            languages: HashMap::new(),
            repo,
            vcs,
        }
    }

    pub fn register_language(&mut self, ext: &str, lang: Arc<dyn LanguageSupport>) {
        self.languages.insert(ext.to_string(), lang.clone());
    }

    pub async fn index_workspace(&self, path: &Path) -> Result<()> {
        for entry in WalkDir::new(path).follow_links(true) {
            let entry = entry?;
            if entry.file_type().is_file() {
                self.index_file(&entry.path()).await?;
            }
        }
        Ok(())
    }

    pub async fn index_file(&self, path: &Path) -> Result<()> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if self.languages.contains_key(ext) {
                let lang = self
                    .languages
                    .get(ext)
                    .ok_or_else(|| anyhow!("failed to get language implementation"))?;
                let parsed = lang
                    .parse(&path)
                    .ok_or_else(|| anyhow!("failed to parse file: {}", path.display()))?;
                let data =
                    self.get_symbols_from_tree(&parsed.0, lang.as_ref(), &path, &parsed.1)?;
                self.repo.insert_symbols(&data.0).await?;

                if !data.1.is_empty() {
                    let mappings = data
                        .1
                        .iter()
                        .map(|mapping| {
                            (
                                &*mapping.symbol_fqn,
                                &*mapping.super_short_name,
                                mapping.super_fqn.as_deref(),
                            )
                        })
                        .collect();

                    self.repo.insert_symbol_super_mappings(mappings).await?;
                }
            }
        }
        Ok(())
    }

    fn get_symbols_from_tree(
        &self,
        tree: &Tree,
        lang: &dyn LanguageSupport,
        path: &Path,
        content: &str,
    ) -> Result<(Vec<Symbol>, Vec<SymbolSuperMapping>)> {
        let mut symbols = Vec::new();
        let mut symbol_super_mappings = Vec::new();
        let package_name = lang
            .get_package_name(tree, content)
            .ok_or_else(|| anyhow!("failed to get package name"))?;

        let imports = lang.get_imports(tree, content);

        self.dfs(
            tree.root_node(),
            lang,
            &package_name,
            false,
            &mut symbols,
            path,
            content,
            &package_name,
            &mut symbol_super_mappings,
            &imports,
        )?;
        Ok((symbols, symbol_super_mappings))
    }

    fn dfs(
        &self,
        node: Node,
        lang: &dyn LanguageSupport,
        parent_name: &str,
        is_type_parent: bool,
        symbols: &mut Vec<Symbol>,
        path: &Path,
        content: &str,
        package_name: &str,
        symbol_super_mappings: &mut Vec<SymbolSuperMapping>,
        imports: &[String],
    ) -> Result<()> {
        let (new_parent, new_is_type_parent) = if lang.should_index(&node) {
            let node_type = lang.get_type(&node);
            let short_name = lang
                .get_short_name(&node, content)
                .context("Failed to get short name")?;
            let sep = if is_type_parent { "#" } else { "." };
            let fqn = format!("{}{}{}", parent_name, sep, short_name);
            let range = lang.get_range(&node).context("Failed to get range")?;
            let ident_range = lang
                .get_ident_range(&node)
                .context("Failed to get ident range")?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("Failed to get duration")?
                .as_secs();

            let modifiers = lang.get_modifiers(&node, content);
            let implements = lang.get_implements(&node, content);

            if let Some(superclass_short_name) = lang.get_extends(&node, content) {
                let superclass_fqn = naive_resolve_fqn(&superclass_short_name, imports);

                symbol_super_mappings.push(SymbolSuperMapping {
                    id: None,
                    symbol_fqn: fqn.clone(),
                    super_short_name: superclass_short_name,
                    super_fqn: superclass_fqn,
                });
            }

            for interface_short_name in implements {
                let interface_fqn = naive_resolve_fqn(&interface_short_name, imports);

                symbol_super_mappings.push(SymbolSuperMapping {
                    id: None,
                    symbol_fqn: fqn.clone(),
                    super_short_name: interface_short_name,
                    super_fqn: interface_fqn,
                });
            }

            let documentation = lang.get_documentation(&node, content);
            let annotations = lang.get_annotations(&node, content);

            let mut metadata = SymbolMetadata {
                annotations: Some(annotations),
                parameters: None,
                documentation: documentation,
                return_type: None,
            };

            match node_type {
                Some(NodeType::Function) => {
                    let symbol_params = lang
                        .get_parameters(&node, content)
                        .context("failed to get function params")?
                        .into_iter()
                        .map(|(name, type_name, default_value)| SymbolParameter {
                            name,
                            type_name,
                            default_value,
                        })
                        .collect();

                    metadata.parameters = Some(symbol_params);

                    metadata.return_type = lang.get_return(&node, content);
                }
                Some(NodeType::Field) => {
                    let ret = lang
                        .get_return(&node, content)
                        .context("failed to get function return type")?;
                    metadata.return_type = Some(ret);
                }
                _ => (),
            };

            symbols.push(Symbol {
                id: None,
                vcs_branch: self.vcs.get_current_branch().ok().unwrap(),
                short_name: short_name,
                package_name: package_name.to_string(),
                fully_qualified_name: fqn.clone(),
                parent_name: Some(parent_name.to_string()),
                file_path: path.to_string_lossy().to_string(),
                file_type: lang.get_language().to_string(),
                symbol_type: node_type.clone().expect("unknown node type").to_string(),
                modifiers: Json::from(modifiers),
                line_start: range.start.line as i64,
                line_end: range.end.line as i64,
                char_start: range.start.character as i64,
                char_end: range.end.character as i64,
                ident_line_start: ident_range.start.line as i64,
                ident_line_end: ident_range.end.line as i64,
                ident_char_start: ident_range.start.character as i64,
                ident_char_end: ident_range.end.character as i64,
                metadata: Json::from(metadata),
                last_modified: now as i64,
            });

            let is_next_type = matches!(
                node_type,
                Some(NodeType::Class | NodeType::Interface | NodeType::Enum)
            );

            (fqn, is_next_type)
        } else {
            (parent_name.to_string(), is_type_parent)
        };

        for child in node.children(&mut node.walk()) {
            self.dfs(
                child,
                lang,
                &new_parent,
                new_is_type_parent,
                symbols,
                path,
                content,
                &package_name,
                symbol_super_mappings,
                imports,
            )?;
        }

        Ok(())
    }

    pub async fn index_jar(&self, jar_path: &Path) -> Result<()> {
        let file = File::open(jar_path)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let entry_name = {
                let entry = archive.by_index(i)?;
                entry.name().to_string()
            };

            if entry_name.ends_with(".java")
                || entry_name.ends_with(".groovy")
                || entry_name.ends_with(".class")
            {
                let mut entry = archive.by_index(i)?;
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;

                let (content, is_decompiled) = if entry_name.ends_with(".class") {
                    (self.decompile_class(buffer).await?, true)
                } else {
                    (String::from_utf8(buffer)?, false)
                };

                self.index_jar_source_content(&content, &entry_name, jar_path, is_decompiled)
                    .await?;
            }
        }

        Ok(())
    }

    async fn decompile_class(&self, buffer: Vec<u8>) -> Result<String> {
        // NOTE: the user should define their own decompile command
        // the decompiled data can then be further integrated to the LSP
        todo!()
    }

    async fn index_jar_source_content(
        &self,
        content: &str,
        entry_name: &str,
        jar_path: &Path,
        is_decompiled: bool,
    ) -> Result<()> {
        let ext = Path::new(entry_name)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow!("No extension"))?;

        if let Some(lang) = self.languages.get(ext) {
            let parsed = lang.parse_str(content).expect("cannot parse content");
            let (external_symbols, mappings) = self.get_external_symbols_from_tree(
                &parsed.0,
                lang.as_ref(),
                jar_path,
                entry_name,
                &parsed.1,
                is_decompiled,
            )?;

            self.repo.insert_external_symbols(&external_symbols).await?;

            if !mappings.is_empty() {
                let mapping_refs = mappings
                    .iter()
                    .map(|m| (&*m.symbol_fqn, &*m.super_short_name, m.super_fqn.as_deref()))
                    .collect();
                self.repo.insert_symbol_super_mappings(mapping_refs).await?;
            }
        }

        Ok(())
    }

    fn get_external_symbols_from_tree(
        &self,
        tree: &Tree,
        lang: &dyn LanguageSupport,
        jar_path: &Path,
        source_file_path: &str,
        content: &str,
        is_decompiled: bool,
    ) -> Result<(Vec<ExternalSymbol>, Vec<SymbolSuperMapping>)> {
        let (symbols, mappings) = self.get_symbols_from_tree(tree, lang, jar_path, content)?;

        let external_symbols = symbols
            .into_iter()
            .map(|s| ExternalSymbol {
                id: None,
                jar_path: jar_path.to_string_lossy().to_string(),
                source_file_path: source_file_path.to_string(),
                short_name: s.short_name,
                fully_qualified_name: s.fully_qualified_name,
                package_name: s.package_name,
                parent_name: s.parent_name,
                symbol_type: s.symbol_type,
                modifiers: s.modifiers,
                line_start: s.line_start,
                line_end: s.line_end,
                char_start: s.char_start,
                char_end: s.char_end,
                ident_line_start: s.ident_line_start,
                ident_line_end: s.ident_line_end,
                ident_char_start: s.ident_char_start,
                ident_char_end: s.ident_char_end,
                is_decompiled,
                metadata: s.metadata,
                last_modified: s.last_modified,
            })
            .collect();

        Ok((external_symbols, mappings))
    }
}
