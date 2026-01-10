use lsp_core::{language_support::LanguageSupport, node_types::NodeType, vcs::handler::VcsHandler};
use std::{collections::HashMap, path::Path, sync::Arc};

use crate::{
    models::symbol::{Symbol, SymbolMetadata, SymbolParameter},
    repo::Repository,
};

use anyhow::{Context, Result, anyhow};
use sqlx::types::Json;
use tree_sitter::{Node, Tree};
use walkdir::WalkDir;

use std::time::{SystemTime, UNIX_EPOCH};

pub struct Indexer {
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
    repo: Arc<Repository>,
    vcs: Box<dyn VcsHandler>,
}

impl Indexer {
    pub fn new(repo: Arc<Repository>, vcs: Box<dyn VcsHandler>) -> Self {
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
                let symbols =
                    self.get_symbols_from_tree(&parsed.0, lang.as_ref(), &path, &parsed.1)?;
                self.repo.insert_symbols(&symbols).await?;
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
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let parent_name = lang
            .get_package_name(tree, content)
            .ok_or_else(|| anyhow!("failed to get package name"))?;
        self.dfs(
            tree.root_node(),
            lang,
            &parent_name,
            false,
            &mut symbols,
            path,
            content,
        )?;
        Ok(symbols)
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
    ) -> Result<()> {
        let node_type = lang.get_type(&node);
        let (new_parent, new_is_type_parent) = if lang.should_index(&node) {
            let short_name = lang
                .get_short_name(&node, content)
                .context("Failed to get short name")?;
            let fqn = match node_type {
                Some(NodeType::Class | NodeType::Interface | NodeType::Enum) => {
                    let sep = if is_type_parent { "$" } else { "." };
                    format!("{}{}{}", parent_name, sep, short_name)
                }
                _ => format!("{}.{}", parent_name, short_name),
            };

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
                extends_name: lang.get_extends(&node, content),
                implements_names: Json::from(implements),
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
            )?;
        }

        Ok(())
    }
}
