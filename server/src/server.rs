use groovy::GroovySupport;
use lsp_core::{
    language_support::LanguageSupport,
    util::capitalize,
    vcs::{get_vcs_handler, handler::VcsHandler},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};
use tokio::sync::RwLock;
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, lsp_types::request::GotoImplementationParams};
use tower_lsp::{jsonrpc::Result, lsp_types::request::GotoImplementationResponse};
use tree_sitter::Tree;

use crate::{Indexer, Repository, models::symbol::Symbol};

pub struct Backend {
    pub client: tower_lsp::Client,
    indexer: Arc<RwLock<Option<Indexer>>>,
    repo: Arc<Repository>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
    vcs_handler: Arc<RwLock<Option<Arc<dyn VcsHandler>>>>,
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
            vcs_handler: Arc::new(RwLock::new(None)),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_fqn(
        &self,
        name: &str,
        imports: Vec<String>,
        package_name: Option<String>,
        branch: &str,
    ) -> Option<String> {
        if name.contains('.') {
            return Some(name.to_string());
        }

        // Direct import match
        if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some(name)) {
            return Some(import.clone());
        }

        // Wildcard import match
        if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some("*")) {
            let tmp_fqn = import.replace("*", name);
            if let Some(_) = self
                .repo
                .find_symbol_by_fqn_and_branch(&tmp_fqn, branch)
                .await
                .ok()?
            {
                return Some(tmp_fqn);
            }
        }

        // Package + name fallback
        return Some(
            package_name
                .map(|pkg| {
                    if !name.contains(&pkg) {
                        format!("{}.{}", pkg, name)
                    } else {
                        name.to_string()
                    }
                })
                .unwrap_or_else(|| name.to_string()),
        );
    }

    #[tracing::instrument(skip_all)]
    async fn try_type_member(
        &self,
        qualifier: &str,
        member: &str,
        imports: &[String],
        package_name: Option<String>,
        branch: &str,
    ) -> Vec<Symbol> {
        let class_fqn = match self
            .resolve_fqn(qualifier, imports.to_vec(), package_name.clone(), branch)
            .await
        {
            Some(fqn) => fqn,
            None => return vec![],
        };

        let mut visited = HashSet::new();
        self.try_members_with_inheritance(
            &class_fqn,
            member,
            branch,
            &mut visited,
            imports.to_vec(),
            package_name,
        )
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn try_property_access(
        &self,
        class_fqn: &str,
        ident: &str,
        branch: &str,
    ) -> Option<Symbol> {
        // Try getter
        let getter_fqn = format!("{}#get{}", class_fqn, capitalize(ident));
        if let Ok(Some(found)) = self
            .repo
            .find_symbol_by_fqn_and_branch(&getter_fqn, branch)
            .await
        {
            return Some(found);
        }

        // Try boolean getter (isX for boolean properties)
        let is_getter_fqn = format!("{}#is{}", class_fqn, capitalize(ident));
        self.repo
            .find_symbol_by_fqn_and_branch(&is_getter_fqn, branch)
            .await
            .ok()
            .flatten()
    }

    #[tracing::instrument(skip(self))]
    async fn try_members_with_inheritance(
        &self,
        type_fqn: &str,
        member: &str,
        branch: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<Symbol> {
        if !visited.insert(type_fqn.to_string()) {
            return vec![];
        }

        // Try direct member
        let member_fqn = format!("{}#{}", type_fqn, member);
        if let Ok(found) = self
            .repo
            .find_symbols_by_fqn_and_branch(&member_fqn, branch)
            .await
        {
            if !found.is_empty() {
                return found;
            }
        }

        // Try property access
        if let Some(found) = self.try_property_access(type_fqn, member, branch).await {
            return vec![found];
        }

        // Get class/interface info to find parents
        let type_symbol = match self
            .repo
            .find_symbol_by_fqn_and_branch(type_fqn, branch)
            .await
        {
            Ok(symbols) => symbols.into_iter().next(),
            Err(_) => None,
        };

        let type_symbol = match type_symbol {
            Some(s) => s,
            None => return vec![],
        };

        // Try superclass/interfaces
        let supers = match self
            .repo
            .get_supers_by_symbol_fqn_and_branch(&type_symbol.fully_qualified_name, &branch)
            .await
        {
            Ok(symbols) => symbols,
            Err(_) => return vec![],
        };

        for super_name in supers.iter().map(|symbol| &symbol.fully_qualified_name) {
            let results = self
                .recurse_try_members_with_inheritance(
                    super_name,
                    member,
                    branch,
                    visited,
                    imports.clone(),
                    package_name.clone(),
                )
                .await;
            if !results.is_empty() {
                return results;
            }
        }

        println!("supers: {:#?}", supers);
        for super_name in supers.iter().map(|symbol| &symbol.fully_qualified_name) {
            let results = self
                .recurse_try_members_with_inheritance(
                    super_name,
                    member,
                    branch,
                    visited,
                    imports.clone(),
                    package_name.clone(),
                )
                .await;
            if !results.is_empty() {
                return results;
            }
        }

        vec![]
    }

    #[tracing::instrument(skip(self))]
    async fn recurse_try_members_with_inheritance(
        &self,
        parent_short_name: &str,
        member: &str,
        branch: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<Symbol> {
        tracing::info!("recurse_try_members_with_inheritance");
        let fqn = match self
            .resolve_fqn(
                &parent_short_name,
                imports.clone(),
                package_name.clone(),
                branch,
            )
            .await
        {
            Some(fqn) => fqn,
            None => return vec![],
        };

        let parent_symbol = match self.repo.find_symbols_by_fqn_and_branch(&fqn, branch).await {
            Ok(symbols) => symbols.into_iter().next(),
            Err(_) => return vec![],
        };

        match parent_symbol {
            Some(parent) => {
                Box::pin(self.try_members_with_inheritance(
                    &parent.fully_qualified_name,
                    member,
                    branch,
                    visited,
                    imports,
                    package_name,
                ))
                .await
            }
            None => vec![],
        }
    }

    #[tracing::instrument(skip_all)]
    async fn symbol_to_defn_response(
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

    fn symbols_to_impl_response(
        &self,
        implementations: Vec<Symbol>,
    ) -> Option<GotoImplementationResponse> {
        let locations: Vec<Location> = implementations
            .into_iter()
            .map(|sym| Location {
                uri: Url::from_file_path(&sym.file_path).unwrap(),
                range: Range {
                    start: Position {
                        line: sym.ident_line_start as u32,
                        character: sym.ident_char_start as u32,
                    },
                    end: Position {
                        line: sym.ident_line_end as u32,
                        character: sym.ident_char_end as u32,
                    },
                },
            })
            .collect();

        let response = match locations.len() {
            0 => None,
            1 => Some(GotoImplementationResponse::Scalar(
                locations.first().unwrap().clone(),
            )),
            _ => Some(GotoImplementationResponse::Array(locations)),
        };

        return response;
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_type_member_chain(
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
    ) -> Vec<Symbol> {
        let parts: Vec<&str> = qualifier.split('#').collect();

        if parts.is_empty() {
            return vec![];
        }

        let base_type =
            if let Some(var_type) = lang.find_variable_type(tree, content, parts[0], position) {
                var_type
            } else {
                parts[0].to_string()
            };

        let mut current_type_fqn = match self
            .resolve_fqn(&base_type, imports.clone(), package_name.clone(), branch)
            .await
        {
            Some(fqn) => fqn,
            None => return vec![],
        };

        if parts.len() > 1 {
            for part in &parts[1..] {
                let symbols = self
                    .try_type_member(&current_type_fqn, part, &imports, None, branch)
                    .await;

                let symbol = match symbols.into_iter().next() {
                    Some(s) => s,
                    None => return vec![],
                };

                current_type_fqn = if let Some(return_type) = &symbol.metadata.return_type {
                    // For methods/fields, resolve their return/field type
                    let parent_package = symbol.package_name.clone();

                    match self
                        .resolve_fqn(return_type, imports.clone(), Some(parent_package), branch)
                        .await
                    {
                        Some(fqn) => fqn,
                        None => return vec![],
                    }
                } else {
                    // For types (Class/Interface/Enum), use their FQN directly
                    symbol.fully_qualified_name.clone()
                };
            }
        }

        // Returns all overloads
        self.try_type_member(&current_type_fqn, member, &imports, None, branch)
            .await
    }

    async fn select_best_overload(
        &self,
        symbols: Vec<Symbol>,
        call_args: Vec<(String, Position)>,
        lang: &Arc<dyn LanguageSupport>,
        tree: &Tree,
        content: &str,
        imports: &[String],
        package_name: Option<String>,
        branch: &str,
    ) -> Option<Symbol> {
        let arg_count = call_args.len();

        let arity_matches: Vec<Symbol> = symbols
            .into_iter()
            .filter(|s| {
                s.metadata
                    .parameters
                    .as_ref()
                    .map_or(false, |params| params.len() == arg_count)
            })
            .collect();

        if arity_matches.len() == 1 {
            return arity_matches.into_iter().next();
        }

        if arity_matches.is_empty() {
            return None;
        }

        let mut arg_fqns = Vec::new();
        for (arg, position) in &call_args {
            let arg_type =
                if let Some(literal_type) = lang.get_literal_type(tree, content, &position) {
                    literal_type
                } else {
                    lang.find_variable_type(tree, content, arg, &position)
                        .unwrap_or_else(|| arg.clone())
                };

            let arg_fqn = self
                .resolve_fqn(&arg_type, imports.to_vec(), package_name.clone(), branch)
                .await
                .unwrap_or(arg_type);

            arg_fqns.push(arg_fqn);
        }

        for symbol in arity_matches {
            let path = PathBuf::from_str(&symbol.file_path).unwrap();

            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let symbol_lang = match self.languages.get(ext) {
                    Some(l) => l,
                    None => {
                        tracing::info!(
                            "failed to get language support for {}. path: {:#?}",
                            ext,
                            path
                        );
                        continue;
                    }
                };

                let (symbol_tree, symbol_content) = match symbol_lang.parse(&path) {
                    Some(p) => p,
                    None => {
                        tracing::info!("failed to parse file {:#?}", path);
                        continue;
                    }
                };

                if let Some(params) = &symbol.metadata.parameters {
                    let mut all_match = true;

                    for (i, param) in params.iter().enumerate() {
                        if let Some(param_type) = &param.type_name {
                            let mut param_type = param_type.to_string();

                            if let Some(top_generic_type) = param_type.split_once('<') {
                                let new_val = top_generic_type.0;
                                param_type = new_val.to_string();
                            }

                            let imports = symbol_lang.get_imports(&symbol_tree, &symbol_content);
                            let param_fqn = self
                                .resolve_fqn(
                                    &param_type,
                                    imports.clone(),
                                    Some(symbol.package_name.clone()),
                                    branch,
                                )
                                .await
                                .unwrap_or(param_type.to_string());

                            if param_fqn != arg_fqns[i] {
                                all_match = false;
                                break;
                            }
                        } else {
                            all_match = false;
                            break;
                        }
                    }

                    if all_match {
                        return Some(symbol);
                    }
                }
            }
        }

        None
    }

    /**
     For cases where matching exact parameter types is impractical/overkill.
    */
    fn filter_by_arity(&self, symbols: Vec<Symbol>, expected_param_count: usize) -> Vec<Symbol> {
        symbols
            .into_iter()
            .filter(|s| {
                s.metadata
                    .parameters
                    .as_ref()
                    .map_or(false, |params| params.len() == expected_param_count)
            })
            .collect()
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
            let mut indexer = Indexer::new(Arc::clone(&self.repo), Arc::clone(&vcs));

            self.languages.iter().for_each(|(k, v)| {
                indexer.register_language(k, v.clone());
            });

            indexer.index_workspace(&root).await.map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "Failed to index workspace: {}",
                    e
                ))
            })?;

            *self.vcs_handler.write().await = Some(vcs);
            *self.workspace_root.write().await = Some(root);
            *self.indexer.write().await = Some(indexer);
        } else {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "No workspace root provided",
            ));
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
        let branch = self
            .vcs_handler
            .read()
            .await
            .as_ref()
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("VCS handler not available"))?
            .get_current_branch()
            .map_err(|e| {
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
                    .resolve_fqn(&type_name, imports, package_name, &branch)
                    .await
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "Failed to find FQN by location",
                    )))?;

                return Ok(Some(self.symbol_to_defn_response(fqn, &branch).await?));
            };

            if let Some((ident, qualifier)) =
                lang.find_ident_at_position(&tree, &content, &position)
            {
                match qualifier {
                    Some(q) => {
                        let symbols = self
                            .resolve_type_member_chain(
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
                            .await;

                        if !symbols.is_empty() {
                            if symbols.len() == 1 {
                                let symbol = symbols.iter().next().unwrap();
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

                            let call_args = lang.extract_call_arguments(&tree, &content, &position);
                            if let Some(args) = call_args {
                                if let Some(symbol) = self
                                    .select_best_overload(
                                        symbols.clone(),
                                        args,
                                        lang,
                                        &tree,
                                        &content,
                                        &imports,
                                        package_name,
                                        &branch,
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
                            }

                            // No call context, return all overloads
                            let locations: Vec<Location> = symbols
                                .into_iter()
                                .filter_map(|s| s.to_lsp_location())
                                .collect();

                            if !locations.is_empty() {
                                return Ok(Some(GotoDefinitionResponse::Array(locations)));
                            }
                        }

                        return Err(tower_lsp::jsonrpc::Error::invalid_params(
                            "Qualifier found but failed to resolve".to_string(),
                        ));
                    }
                    None => {
                        let fqn = self
                            .resolve_fqn(&ident, imports, package_name, &branch)
                            .await
                            .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                                "Failed to find FQN by location",
                            )))?;

                        return Ok(Some(self.symbol_to_defn_response(fqn, &branch).await?));
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

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let branch = self
            .vcs_handler
            .read()
            .await
            .as_ref()
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("VCS handler not available"))?
            .get_current_branch()
            .map_err(|e| {
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

            if let Some((ident, _)) = lang.find_ident_at_position(&tree, &content, &position) {
                if let Some(type_name) =
                    lang.get_type_at_position(tree.root_node(), &content, &position)
                {
                    let fqn = self
                        .resolve_fqn(&type_name, imports, package_name, &branch)
                        .await
                        .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                            "Failed to find FQN by location",
                        )))?;

                    let implementations = self
                        .repo
                        .get_super_impls_by_fqn_and_branch(&fqn, &branch)
                        .await
                        .map_err(|e| {
                            tower_lsp::jsonrpc::Error::invalid_params(format!(
                                "Failed to find parent implementations by FQN: {}",
                                e,
                            ))
                        })?;

                    let implementations = if implementations.is_empty() {
                        // Best effort
                        self.repo
                            .get_super_impls_by_short_name_and_branch(&type_name, &branch)
                            .await
                            .map_err(|e| {
                                tower_lsp::jsonrpc::Error::invalid_params(format!(
                                    "Failed to find parent implementations by short name: {}",
                                    e,
                                ))
                            })?
                    } else {
                        implementations
                    };

                    return Ok(self.symbols_to_impl_response(implementations));
                };

                if let Some((receiver_type, params)) =
                    lang.get_method_receiver_and_params(tree.root_node(), &content, &position)
                {
                    let parent_fqn = self
                        .resolve_fqn(&receiver_type, imports, package_name, &branch)
                        .await
                        .ok_or_else(|| {
                            tower_lsp::jsonrpc::Error::invalid_params("Failed to resolve FQN")
                        })?;

                    let implementations = self
                        .repo
                        .get_super_impls_by_fqn_and_branch(&parent_fqn, &branch)
                        .await
                        .map_err(|e| {
                            tower_lsp::jsonrpc::Error::invalid_params(format!(
                                "Failed to find parent implementations by FQN: {}",
                                e,
                            ))
                        })?;

                    let mut method_symbols = Vec::new();
                    for impl_symbol in &implementations {
                        let method_fqn = format!("{}#{}", impl_symbol.fully_qualified_name, &ident);

                        if let Ok(symbols) = self
                            .repo
                            .find_symbols_by_fqn_and_branch(&method_fqn, &branch)
                            .await
                        {
                            method_symbols.extend(symbols);
                        }
                    }

                    method_symbols = self.filter_by_arity(method_symbols, params.len());

                    return Ok(self.symbols_to_impl_response(method_symbols));
                }
            }
        }

        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let file_path = match params.text_document.uri.to_file_path() {
            Ok(path) => path,
            Err(_) => return,
        };

        if let Some(indexer) = self.indexer.read().await.as_ref() {
            if let Err(e) = indexer.index_file(&file_path).await {
                tracing::error!("Failed to index file {:?}: {}", &file_path, e);
            }
        }
    }
}
