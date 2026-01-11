use groovy::GroovySupport;
use lsp_core::{language_support::LanguageSupport, util::capitalize, vcs::get_vcs_handler};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};
use tokio::sync::RwLock;
use tower_lsp::LanguageServer;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tree_sitter::Tree;

use crate::{Indexer, Repository, models::symbol::Symbol};

pub struct Backend {
    pub client: tower_lsp::Client,
    indexer: Arc<RwLock<Option<Indexer>>>,
    repo: Arc<Repository>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
}

impl Backend {
    pub fn new(client: tower_lsp::Client, repo: Arc<Repository>) -> Self {
        let mut languages: HashMap<String, Arc<dyn LanguageSupport>> = HashMap::new();
        languages.insert("groovy".to_string(), Arc::new(GroovySupport::new()));

        Self {
            client,
            indexer: Arc::new(RwLock::new(None)),
            repo,
            workspace_root: Arc::new(RwLock::new(None)),
            languages,
        }
    }

    async fn resolve_fqn(
        &self,
        name: &str,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Option<String> {
        // Direct import match
        if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some(name)) {
            return Some(import.clone());
        }

        // Wildcard import match
        if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some("*")) {
            let tmp_fqn = import.replace("*", name);
            if let Some(_) = self.repo.find_symbol_by_fqn(&tmp_fqn).await.ok()? {
                return Some(tmp_fqn);
            }
        }

        // Package + name fallback
        Some(
            package_name
                .map(|pkg| format!("{}.{}", pkg, name))
                .unwrap_or_else(|| name.to_string()),
        )
    }

    async fn try_static_member(
        &self,
        qualifier: &str,
        ident: &str,
        imports: &[String],
        package_name: &Option<String>,
        branch: &str,
    ) -> Option<Symbol> {
        let class_fqn = self
            .resolve_fqn(qualifier, imports.to_vec(), package_name.clone())
            .await?;
        let member_fqn = format!("{}.{}", class_fqn, ident);

        if let Ok(Some(found)) = self
            .repo
            .find_symbol_by_fqn_and_branch(&member_fqn, branch)
            .await
        {
            return Some(found);
        }

        self.try_property_access(&class_fqn, ident, branch).await
    }

    async fn try_property_access(
        &self,
        class_fqn: &str,
        ident: &str,
        branch: &str,
    ) -> Option<Symbol> {
        // Try getter
        let getter_fqn = format!("{}.get{}", class_fqn, capitalize(ident));
        if let Ok(Some(found)) = self
            .repo
            .find_symbol_by_fqn_and_branch(&getter_fqn, branch)
            .await
        {
            return Some(found);
        }

        // Try boolean getter (isX for boolean properties)
        let is_getter_fqn = format!("{}.is{}", class_fqn, capitalize(ident));
        self.repo
            .find_symbol_by_fqn_and_branch(&is_getter_fqn, branch)
            .await
            .ok()
            .flatten()
    }

    async fn try_instance_member(
        &self,
        qualifier: &str,
        member: &str,
        lang: &Arc<dyn LanguageSupport>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        branch: &str,
        position: &Position,
        package_name: Option<String>,
    ) -> Option<Symbol> {
        let var_type = lang.find_variable_type(tree, content, qualifier, position)?;

        let type_fqn = self
            .resolve_fqn(&var_type, imports.clone(), package_name.clone())
            .await?;

        let mut visited = HashSet::new();
        self.try_member_with_inheritance(
            &type_fqn,
            member,
            branch,
            &mut visited,
            imports,
            &package_name,
        )
        .await
    }

    async fn try_member_with_inheritance(
        &self,
        type_fqn: &str,
        member: &str,
        branch: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: &Option<String>,
    ) -> Option<Symbol> {
        if !visited.insert(type_fqn.to_string()) {
            return None;
        }

        // Try direct member
        let member_fqn = format!("{}.{}", type_fqn, member);

        if let Ok(Some(found)) = self
            .repo
            .find_symbol_by_fqn_and_branch(&member_fqn, branch)
            .await
        {
            return Some(found);
        }

        // Try property access
        if let Some(found) = self.try_property_access(type_fqn, member, branch).await {
            return Some(found);
        }

        // Get class/interface info to find parents
        let type_symbol = self
            .repo
            .find_symbol_by_fqn_and_branch(type_fqn, branch)
            .await
            .ok()??;

        // Try superclass
        if let Some(super_name) = type_symbol.extends_name {
            if let Some(symbol) = self
                .recurse_try_member_with_inheritance(
                    &super_name,
                    member,
                    branch,
                    visited,
                    imports.clone(),
                    package_name,
                )
                .await
            {
                return Some(symbol);
            }
        }

        // Try interfaces
        if !type_symbol.implements_names.0.is_empty() {
            for super_name in &type_symbol.implements_names.0 {
                if let Some(symbol) = self
                    .recurse_try_member_with_inheritance(
                        &super_name,
                        member,
                        branch,
                        visited,
                        imports.clone(),
                        package_name,
                    )
                    .await
                {
                    return Some(symbol);
                }
            }
        }

        None
    }

    async fn recurse_try_member_with_inheritance(
        &self,
        parent_short_name: &str,
        member: &str,
        branch: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: &Option<String>,
    ) -> Option<Symbol> {
        let fqn = self
            .resolve_fqn(&parent_short_name, imports.clone(), package_name.clone())
            .await
            .unwrap_or_default();
        if let Some(parent_symbol) = self
            .repo
            .find_symbol_by_fqn_and_branch(&fqn, branch)
            .await
            .ok()?
        {
            if let Some(symbol) = Box::pin(self.try_member_with_inheritance(
                &parent_symbol.fully_qualified_name,
                member,
                branch,
                visited,
                imports,
                package_name,
            ))
            .await
            {
                return Some(symbol);
            }
        }
        None
    }

    async fn symbol_to_response(
        &self,
        fqn: String,
        branch: &str,
    ) -> Result<GotoDefinitionResponse> {
        let symbol = self
            .repo
            .find_symbol_by_fqn_and_branch(&fqn, branch)
            .await
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("Failed to find symbol: {}", e))
            })?
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params("Symbol not found".to_string())
            })?;

        symbol
            .to_lsp_location()
            .map(GotoDefinitionResponse::from)
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(
                    "Failed to convert to location".to_string(),
                )
            })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let workspace_root = params
            .root_uri
            .and_then(|uri| uri.to_file_path().ok())
            .or_else(|| {
                params
                    .workspace_folders
                    .and_then(|folders| folders.first().cloned())
                    .and_then(|folder| folder.uri.to_file_path().ok())
            });

        if let Some(root) = workspace_root {
            let vcs = get_vcs_handler(&root);
            let mut indexer = Indexer::new(Arc::clone(&self.repo), vcs);

            self.languages.iter().for_each(|(k, v)| {
                indexer.register_language(k, v.clone());
            });

            indexer.index_workspace(&root).await.map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "Failed to index workspace: {}",
                    e
                ))
            })?;

            *self.workspace_root.write().await = Some(root);
            *self.indexer.write().await = Some(indexer);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions::default()),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "lspintar".to_string(),
                version: Some("0.1.0".to_string()),
            }),
        })
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let root = self
            .workspace_root
            .read()
            .await
            .clone()
            .ok_or(tower_lsp::jsonrpc::Error::invalid_request())?;

        let vcs = get_vcs_handler(&root);
        let branch = vcs.get_current_branch().map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "Failed to get current branch: {}",
                e
            ))
        })?;

        let path = PathBuf::from_str(
            params
                .text_document_position_params
                .text_document
                .uri
                .path(),
        )
        .unwrap();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lang = self.languages.get(ext).ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(
                    format!("Failed to get language support",),
                )
            })?;

            let (tree, content) = lang.parse(&path).ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("Failed to parse file"))
            })?;

            let imports = lang.get_imports(&tree, &content);
            let package_name = lang.get_package_name(&tree, &content);

            let position = params.text_document_position_params.position;

            if let Some(type_name) =
                lang.get_type_at_position(tree.root_node(), &content, &position)
            {
                let fqn = self
                    .resolve_fqn(&type_name, imports, package_name)
                    .await
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "Failed to find FQN by location",
                    )))?;

                return Ok(Some(self.symbol_to_response(fqn, &branch).await?));
            };

            if let Some((ident, qualifier)) =
                lang.find_ident_at_position(&tree, &content, &position)
            {
                match qualifier {
                    Some(q) => {
                        if let Some(symbol) = self
                            .try_static_member(&q, &ident, &imports, &package_name, &branch)
                            .await
                        {
                            return Ok(Some(
                                symbol
                                    .to_lsp_location()
                                    .map(GotoDefinitionResponse::from)
                                    .ok_or_else(|| {
                                        tower_lsp::jsonrpc::Error::invalid_params(
                                            "Failed to convert to location".to_string(),
                                        )
                                    })?,
                            ));
                        }

                        if let Some(symbol) = self
                            .try_instance_member(
                                &q,
                                &ident,
                                &lang,
                                &tree,
                                &content,
                                imports.clone(),
                                &branch,
                                &position,
                                package_name.clone(),
                            )
                            .await
                        {
                            return Ok(Some(
                                symbol
                                    .to_lsp_location()
                                    .map(GotoDefinitionResponse::from)
                                    .ok_or_else(|| {
                                        tower_lsp::jsonrpc::Error::invalid_params(
                                            "Failed to convert to location".to_string(),
                                        )
                                    })?,
                            ));
                        }

                        if let Some(symbol) = self
                            .try_instance_member(
                                &q,
                                &ident,
                                &lang,
                                &tree,
                                &content,
                                imports,
                                &branch,
                                &position,
                                package_name,
                            )
                            .await
                        {
                            return Ok(Some(
                                symbol
                                    .to_lsp_location()
                                    .map(GotoDefinitionResponse::from)
                                    .ok_or_else(|| {
                                        tower_lsp::jsonrpc::Error::invalid_params(
                                            "Failed to convert to location".to_string(),
                                        )
                                    })?,
                            ));
                        }

                        // Strategy 4: Handle chained calls (advanced - maybe defer)
                        // Examples: user.getProfile().getName()
                        // - Would need to evaluate left-to-right
                        // - Find type of "user", then return type of "getProfile()", etc.
                        // - This is complex, consider skipping for MVP

                        return Err(tower_lsp::jsonrpc::Error::invalid_params(
                            "Qualifier found but failed to resolve".to_string(),
                        ));
                    }
                    None => {
                        let fqn = self
                            .resolve_fqn(&ident, imports, package_name)
                            .await
                            .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                                "Failed to find FQN by location",
                            )))?;

                        return Ok(Some(self.symbol_to_response(fqn, &branch).await?));
                    }
                }
            } else {
                return Err(tower_lsp::jsonrpc::Error::invalid_params(
                    "Failed to get ident/type name".to_string(),
                ));
            };
        };

        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let file_path = match params.text_document.uri.to_file_path() {
            Ok(path) => path,
            Err(_) => return,
        };

        if let Some(indexer) = self.indexer.read().await.as_ref() {
            if let Err(e) = indexer.index_file(&file_path).await {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "Failed to index file {:?}: {}",
                    &file_path, e
                ));
            }
        }
    }
}
