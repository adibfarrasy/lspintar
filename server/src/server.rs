use core::panic;
use futures::{StreamExt, stream};
use groovy::GroovySupport;
use java::JavaSupport;
use kotlin::KotlinSupport;
use lsp_core::{
    build_tools::get_build_tool,
    language_support::LanguageSupport,
    lsp_error, lsp_info, lsp_logging, lsp_progress, lsp_progress_begin, lsp_progress_end,
    util::capitalize,
    vcs::{VcsHandler, get_vcs_handler},
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};
use tokio::sync::RwLock;
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, lsp_types::request::GotoImplementationParams};
use tower_lsp::{jsonrpc::Result, lsp_types::request::GotoImplementationResponse};
use tracing::debug;
use tree_sitter::Tree;

use crate::{
    Indexer, Repository,
    enums::ResolvedSymbol,
    lsp_convert::{AsLspHover, AsLspLocation},
    models::symbol::Symbol,
};

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
        lsp_logging::init_logging_service(client.clone());

        let mut languages: HashMap<String, Arc<dyn LanguageSupport>> = HashMap::new();
        languages.insert("groovy".to_string(), Arc::new(GroovySupport::new()));
        languages.insert("java".to_string(), Arc::new(JavaSupport::new()));
        languages.insert("kt".to_string(), Arc::new(KotlinSupport::new()));

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
        for import in imports.iter().filter(|i| i.ends_with(".*")) {
            let tmp_fqn = import.replace("*", name);
            if let Some(_) = self
                .repo
                .find_symbol_by_fqn_and_branch(&tmp_fqn, branch)
                .await
                .ok()?
            {
                return Some(tmp_fqn);
            }
            if let Ok(Some(_)) = self.repo.find_external_symbol_by_fqn(&tmp_fqn).await {
                return Some(tmp_fqn);
            }
        }

        // Package + name fallback
        let fallback_fqn = package_name
            .map(|pkg| {
                if !name.contains(&pkg) {
                    format!("{}.{}", pkg, name)
                } else {
                    name.to_string()
                }
            })
            .unwrap_or_else(|| name.to_string());

        if let Ok(Some(_)) = self.repo.find_external_symbol_by_fqn(&fallback_fqn).await {
            return Some(fallback_fqn);
        }

        Some(fallback_fqn)
    }

    #[tracing::instrument(skip_all)]
    async fn try_type_member(
        &self,
        qualifier: &str,
        member: &str,
        imports: &[String],
        package_name: Option<String>,
        branch: &str,
    ) -> Vec<ResolvedSymbol> {
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

    async fn try_parent_member(
        &self,
        type_fqn: &str,
        member: &str,
        branch: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
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

        let supers = match self
            .repo
            .find_supers_by_symbol_fqn_and_branch(&type_symbol.fully_qualified_name, &branch)
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

        vec![]
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
    ) -> Vec<ResolvedSymbol> {
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
                return found.into_iter().map(ResolvedSymbol::Project).collect();
            }
        }

        if let Some(found) = self.try_property_access(type_fqn, member, branch).await {
            return vec![ResolvedSymbol::Project(found)];
        }

        let result = self
            .try_parent_member(type_fqn, member, branch, visited, imports, package_name)
            .await;
        if !result.is_empty() {
            return result;
        }

        if let Ok(Some(found)) = self.repo.find_external_symbol_by_fqn(&member_fqn).await {
            return vec![ResolvedSymbol::External(found)];
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
    ) -> Vec<ResolvedSymbol> {
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

    fn resolved_symbols_to_impl_response(
        &self,
        implementations: Vec<ResolvedSymbol>,
    ) -> Option<GotoImplementationResponse> {
        let locations: Vec<Location> = implementations
            .into_iter()
            .filter_map(|sym| sym.as_lsp_location())
            .collect();

        match locations.len() {
            0 => None,
            1 => Some(GotoImplementationResponse::Scalar(
                locations.into_iter().next().unwrap(),
            )),
            _ => Some(GotoImplementationResponse::Array(locations)),
        }
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
    ) -> Vec<ResolvedSymbol> {
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
                let resolved = match symbols.into_iter().next() {
                    Some(s) => s,
                    None => return vec![],
                };

                current_type_fqn = if let Some(return_type) =
                    resolved.metadata().and_then(|m| m.return_type.as_ref())
                {
                    // For methods/fields, resolve their return/field type
                    let parent_package = resolved.package_name().unwrap_or_default().to_string();
                    match self
                        .resolve_fqn(return_type, imports.clone(), Some(parent_package), branch)
                        .await
                    {
                        Some(fqn) => fqn,
                        None => return vec![],
                    }
                } else {
                    // For types (Class/Interface/Enum), use their FQN directly
                    resolved.package_name().unwrap_or_default().to_string()
                };
            }
        }
        // Returns all overloads
        self.try_type_member(&current_type_fqn, member, &imports, None, branch)
            .await
    }

    async fn select_best_overload(
        &self,
        symbols: Vec<ResolvedSymbol>,
        call_args: Vec<(String, Position)>,
        lang: &Arc<dyn LanguageSupport>,
        tree: &Tree,
        content: &str,
        imports: &[String],
        package_name: Option<String>,
        branch: &str,
    ) -> Option<ResolvedSymbol> {
        let arg_count = call_args.len();

        let arity_matches: Vec<ResolvedSymbol> = symbols
            .into_iter()
            .filter(|s| {
                s.metadata()
                    .and_then(|m| m.parameters.as_ref())
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

        for resolved in arity_matches {
            let params = &resolved.metadata().and_then(|m| m.parameters.as_ref());
            let pkg_name = resolved.package_name().unwrap_or_default();

            if let Some(params) = params {
                let mut all_match = true;
                for (i, param) in params.iter().enumerate() {
                    if let Some(param_type) = &param.type_name {
                        let mut param_type = param_type.to_string();
                        if let Some(top_generic_type) = param_type.split_once('<') {
                            param_type = top_generic_type.0.to_string();
                        }

                        let param_fqn = self
                            .resolve_fqn(
                                &param_type,
                                imports.to_vec(),
                                Some(pkg_name.to_string()),
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
                    return Some(resolved);
                }
            }
        }

        None
    }

    /**
     For cases where matching exact parameter types is impractical/overkill.
    */
    fn filter_by_arity(
        &self,
        symbols: Vec<ResolvedSymbol>,
        expected_param_count: usize,
    ) -> Vec<ResolvedSymbol> {
        symbols
            .into_iter()
            .filter(|s| match s {
                ResolvedSymbol::Project(symbol) => symbol
                    .metadata
                    .parameters
                    .as_ref()
                    .map_or(false, |params| params.len() == expected_param_count),
                ResolvedSymbol::External(external) => external
                    .metadata
                    .parameters
                    .as_ref()
                    .map_or(false, |params| params.len() == expected_param_count),
                ResolvedSymbol::Local { .. } => false,
            })
            .collect()
    }

    async fn resolve_symbol_at_position(
        &self,
        params: &TextDocumentPositionParams,
        branch: &str,
    ) -> Result<Vec<ResolvedSymbol>> {
        let path = PathBuf::from_str(params.text_document.uri.path()).unwrap();

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("No file extension"))?;

        let lang = self.languages.get(ext).ok_or_else(|| {
            tower_lsp::jsonrpc::Error::invalid_params("Failed to get language support")
        })?;

        let (tree, content) = lang
            .parse(&path)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Failed to parse file"))?;

        let imports = lang.get_imports(&tree, &content);
        let package_name = lang.get_package_name(&tree, &content);
        let position = params.position;

        if let Some(type_name) = lang.get_type_at_position(tree.root_node(), &content, &position) {
            let fqn = self
                .resolve_fqn(&type_name, imports, package_name, branch)
                .await
                .ok_or_else(|| {
                    tower_lsp::jsonrpc::Error::invalid_params("Failed to find FQN by location")
                })?;

            return self.fqn_to_symbols(fqn, branch).await;
        }

        if let Some((ident, qualifier)) = lang.find_ident_at_position(&tree, &content, &position) {
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
                            branch,
                            &position,
                            package_name.clone(),
                        )
                        .await;

                    if symbols.is_empty() {
                        return Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
                            "Qualifier {q} found but failed to resolve"
                        )));
                    }

                    if symbols.len() == 1 {
                        return Ok(symbols);
                    }

                    if let Some(args) = lang.extract_call_arguments(&tree, &content, &position) {
                        if let Some(symbol) = self
                            .select_best_overload(
                                symbols.clone(),
                                args,
                                lang,
                                &tree,
                                &content,
                                &imports,
                                package_name,
                                branch,
                            )
                            .await
                        {
                            return Ok(vec![symbol]);
                        }
                    }

                    Ok(symbols)
                }
                None => {
                    if let Some((var_type, var_pos)) =
                        lang.find_variable_declaration(&tree, &content, &ident, &position)
                    {
                        return Ok(vec![ResolvedSymbol::Local {
                            name: ident.clone(),
                            var_type,
                            uri: params.text_document.uri.clone(),
                            position: var_pos,
                        }]);
                    }

                    let fqn = self
                        .resolve_fqn(&ident, imports, package_name, branch)
                        .await
                        .ok_or_else(|| {
                            tower_lsp::jsonrpc::Error::invalid_params(
                                "Failed to find FQN by location",
                            )
                        })?;

                    self.fqn_to_symbols(fqn, branch).await
                }
            }
        } else {
            Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Failed to get ident/type name",
            ))
        }
    }

    #[tracing::instrument(skip_all)]
    async fn fqn_to_symbols(&self, fqn: String, branch: &str) -> Result<Vec<ResolvedSymbol>> {
        if let Ok(Some(symbol)) = self.repo.find_symbol_by_fqn_and_branch(&fqn, branch).await {
            return Ok(vec![ResolvedSymbol::Project(symbol)]);
        }

        let external_symbol = self
            .repo
            .find_external_symbol_by_fqn(&fqn)
            .await
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("Failed to find symbol: {}", e))
            })?
            .ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("Symbol not found for {}", fqn))
            })?;
        Ok(vec![ResolvedSymbol::External(external_symbol)])
    }

    fn is_cache_dir(&self, path: Option<&Path>) -> bool {
        path.map(|p| {
            return p
                .components()
                .any(|c| matches!(c.as_os_str().to_str(), Some(".gradle" | ".m2" | "caches")));
        });

        false
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
            if self.is_cache_dir(Some(&root)) {
                debug!("not a project directory, shutting down: {:?}", root);
                std::process::exit(0);
            }

            *self.workspace_root.write().await = Some(root);
        } else {
            debug!("workspace root not found, shutting down");
            std::process::exit(0);
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

    async fn initialized(&self, _: InitializedParams) {
        let workspace_root = self.workspace_root.read().await.clone();

        if let Some(root) = workspace_root {
            let repo = Arc::clone(&self.repo);
            let indexer_lock = Arc::clone(&self.indexer);
            let vcs_handler_lock = Arc::clone(&self.vcs_handler);
            let workspace_root_lock = Arc::clone(&self.workspace_root);
            let languages: Vec<_> = self
                .languages
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            let vcs = get_vcs_handler(&root);
            let build_tool = get_build_tool(&root);

            let mut indexer = Indexer::new(Arc::clone(&repo), Arc::clone(&vcs));
            languages.iter().for_each(|(k, v)| {
                indexer.register_language(k, v.clone());
            });

            let indexing_start = Instant::now();

            lsp_info!("Resolving dependencies...");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let external_deps = match build_tool.get_dependency_paths(&root) {
                Ok(deps) => deps,
                Err(e) => {
                    let message = format!("Failed to get dependencies: {e}");
                    lsp_error!("{}", message);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    panic!("{}", message);
                }
            };
            let jdk_sources = match build_tool.get_jdk_dependency_path(&root) {
                Ok(deps) => deps,
                Err(e) => {
                    let message = format!("Failed to get JDK sources: {e}");
                    lsp_error!("{}", message);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    panic!("{}", message);
                }
            };
            let mut jars: Vec<_> = external_deps;
            if let Some(src_zip) = jdk_sources {
                jars.push((None, Some(src_zip)));
            }

            let token_ws = format!("idx-ws-{}", uuid::Uuid::new_v4());
            let token_ws_end = token_ws.clone();

            lsp_progress_begin!(&token_ws, "Indexing...");

            let ws_result = indexer
                .index_workspace(&root, move |completed, total| {
                    lsp_progress!(
                        &token_ws,
                        &format!("(1/2) Indexing workspace ({}/{})", completed, total),
                        (completed as f32 / total as f32) * 100.0
                    );
                    if completed == total {
                        lsp_progress_end!(&token_ws_end);
                    }
                })
                .await;

            if let Err(e) = ws_result {
                let message = format!("Failed to index workspace: {e}");
                lsp_error!("{}", message);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                panic!("{}", message);
            }

            let token_jar = format!("idx-ext-{}", uuid::Uuid::new_v4());
            let token_jar_end = token_jar.clone();

            lsp_progress_begin!(&token_jar, "Indexing...");

            indexer
                .index_external_deps(jars, move |completed, total| {
                    lsp_progress!(
                        &token_jar,
                        &format!("(2/2) Indexing JARs ({}/{})", completed, total),
                        (completed as f32 / total as f32) * 100.0
                    );
                    if completed == total {
                        lsp_progress_end!(&token_jar_end);
                    }
                })
                .await;

            lsp_info!(
                "Indexing finished in {:.2}s",
                indexing_start.elapsed().as_secs_f64()
            );

            *indexer_lock.write().await = Some(indexer);
            *vcs_handler_lock.write().await = Some(vcs);
            *workspace_root_lock.write().await = Some(root);
        }
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

        let symbols = self
            .resolve_symbol_at_position(&params.text_document_position_params, &branch)
            .await?;

        let locations: Vec<Location> = symbols
            .into_iter()
            .filter_map(|s| s.as_lsp_location())
            .collect();
        match locations.len() {
            0 => Ok(None),
            1 => Ok(Some(GotoDefinitionResponse::from(
                locations.into_iter().next().unwrap(),
            ))),
            _ => Ok(Some(GotoDefinitionResponse::Array(locations))),
        }
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
                        .find_super_impls_by_fqn_and_branch(&fqn, &branch)
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
                            .find_super_impls_by_short_name_and_branch(&type_name, &branch)
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

                    return Ok(self.resolved_symbols_to_impl_response(
                        implementations
                            .into_iter()
                            .map(|i| ResolvedSymbol::Project(i))
                            .collect(),
                    ));
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
                        .find_super_impls_by_fqn_and_branch(&parent_fqn, &branch)
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
                            let resolved: Vec<ResolvedSymbol> = symbols
                                .into_iter()
                                .map(|s| ResolvedSymbol::Project(s))
                                .collect();

                            method_symbols.extend(resolved);
                        }
                    }

                    method_symbols = self.filter_by_arity(method_symbols, params.len());

                    return Ok(self.resolved_symbols_to_impl_response(method_symbols));
                }
            }
        }

        Ok(None)
    }

    #[tracing::instrument(skip_all)]
    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
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

        let symbols = self
            .resolve_symbol_at_position(&params.text_document_position_params, &branch)
            .await;

        let Ok(symbols) = symbols else {
            return Ok(None);
        };

        let symbol = symbols
            .into_iter()
            .find(|s| !matches!(s, ResolvedSymbol::Local { .. }));

        let Some(symbol) = symbol else {
            return Ok(None);
        };

        Ok(symbol.as_lsp_hover())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {}
}
