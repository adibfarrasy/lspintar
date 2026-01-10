use groovy::GroovySupport;
use lsp_core::{language_support::LanguageSupport, vcs::get_vcs_handler};
use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};
use tokio::sync::RwLock;
use tower_lsp::LanguageServer;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

use crate::{Indexer, Repository};

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
            let name = lang
                .get_type_at_position(&tree, &content, &position)
                .or_else(|| {
                    lang.get_ident_at_position(&tree, &content, &position)
                        .and_then(|(ident, qualifier)| {
                            match qualifier {
                                Some(q) => {
                                    // if let Some(class_fqn) = self
                                    //     .resolve_fqn(
                                    //         &qualifier,
                                    //         imports.clone(),
                                    //         package_name.clone(),
                                    //     )
                                    //     .await
                                    // {
                                    //     let member_fqn = format!("{}.{}", class_fqn, ident);
                                    //     // Check if this member exists
                                    //     if self
                                    //         .repo
                                    //         .find_symbol_by_fqn_and_branch(&member_fqn, &branch)
                                    //         .await
                                    //         .ok()
                                    //         .flatten()
                                    //         .is_some()
                                    //     {
                                    //         return Some(member_fqn);
                                    //     }
                                    // }
                                    //
                                    // // Try 2: Resolve qualifier as a variable (instance access)
                                    // if let Some(var_type) =
                                    //     lang.find_variable_type(&tree, &content, &qualifier)
                                    // {
                                    //     if let Some(type_fqn) = self
                                    //         .resolve_fqn(
                                    //             &var_type,
                                    //             imports.clone(),
                                    //             package_name.clone(),
                                    //         )
                                    //         .await
                                    //     {
                                    //         let member_fqn = format!("{}.{}", type_fqn, ident);
                                    //         if self
                                    //             .repo
                                    //             .find_symbol_by_fqn_and_branch(&member_fqn, &branch)
                                    //             .await
                                    //             .ok()
                                    //             .flatten()
                                    //             .is_some()
                                    //         {
                                    //             return Some(member_fqn);
                                    //         }
                                    //     }
                                    // }

                                    None
                                }
                                None => Some(ident),
                            }
                        })
                })
                .ok_or_else(|| {
                    tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "Failed to get ident/type name",
                    ))
                })?;

            let fqn = self.resolve_fqn(&name, imports, package_name).await.ok_or(
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "Failed to find FQN by location",
                )),
            )?;

            let symbol = self
                .repo
                .find_symbol_by_fqn_and_branch(&fqn, &branch)
                .await
                .map_err(|e| {
                    tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "Failed to find symbol by fqn and branch: {}",
                        e
                    ))
                })?;

            tracing::debug!("symbol: {:#?}", symbol);

            return Ok(symbol
                .and_then(|s| s.to_lsp_location())
                .map(GotoDefinitionResponse::from));
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
