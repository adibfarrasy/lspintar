use core::panic;
use dashmap::DashMap;
use groovy::GroovySupport;
use java::JavaSupport;
use kotlin::KotlinSupport;
use lsp_core::{
    build_tools::{BuildToolHandler, get_build_tool},
    language_support::LanguageSupport,
    lsp_error, lsp_info, lsp_logging, lsp_progress, lsp_progress_begin, lsp_progress_end,
    util::{capitalize, extract_prefix, extract_receiver, get_import_text_edit},
    vcs::{VcsHandler, get_vcs_handler},
};
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{OnceCell, RwLock};
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, lsp_types::request::GotoImplementationParams};
use tower_lsp::{jsonrpc::Result, lsp_types::request::GotoImplementationResponse};
use tracing::{debug, warn};
use tree_sitter::Tree;

use crate::{
    Indexer, Repository,
    constants::{
        DB_PATH_FRAGMENT, FILE_CACHE_TTL_SECS, INDEX_PATH_FRAGMENT, INDEX_VERSION,
        MANIFEST_PATH_FRAGMENT,
    },
    enums::ResolvedSymbol,
    lsp_convert::{AsLspHover, AsLspLocation},
    models::symbol::Symbol,
};

#[derive(Clone)]
pub struct Backend {
    // used in tests
    #[allow(dead_code)]
    pub client: tower_lsp::Client,
    pub repo: OnceCell<Arc<Repository>>,

    indexer: Arc<RwLock<Option<Indexer>>>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    languages: HashMap<String, Arc<dyn LanguageSupport + Send + Sync>>,
    vcs_handler: Arc<RwLock<Option<Arc<dyn VcsHandler + Send + Sync>>>>,
    last_known_revision: Arc<RwLock<Option<String>>>,
    build_tool: Arc<RwLock<Option<Arc<dyn BuildToolHandler + Send + Sync>>>>,

    // Optimizations
    /// Caches open document contents to avoid excessive I/O reads.
    documents: DashMap<String, (String, Instant)>,
    /// Debounces `didChangeWatchedFiles` to avoid redundant reindexing.
    debounce_tx: tokio::sync::mpsc::Sender<PathBuf>,
}

impl Backend {
    pub fn new(client: tower_lsp::Client) -> Self {
        lsp_logging::init_logging_service(client.clone());

        let mut languages: HashMap<String, Arc<dyn LanguageSupport + Send + Sync>> = HashMap::new();
        languages.insert("groovy".to_string(), Arc::new(GroovySupport::new()));
        languages.insert("java".to_string(), Arc::new(JavaSupport::new()));
        languages.insert("kt".to_string(), Arc::new(KotlinSupport::new()));

        let (debounce_tx, debounce_rx) = tokio::sync::mpsc::channel::<PathBuf>(64);
        let backend = Self {
            client,
            indexer: Arc::new(RwLock::new(None)),
            repo: OnceCell::new(),
            workspace_root: Arc::new(RwLock::new(None)),
            languages,
            vcs_handler: Arc::new(RwLock::new(None)),
            last_known_revision: Arc::new(RwLock::new(None)),
            build_tool: Arc::new(RwLock::new(None)),
            documents: DashMap::new(),
            debounce_tx,
        };

        backend.spawn_debounce_task(debounce_rx);
        backend
    }

    fn spawn_debounce_task(&self, mut debounce_rx: tokio::sync::mpsc::Receiver<PathBuf>) {
        let indexer = Arc::clone(&self.indexer);
        let repo = self.repo.clone();

        tokio::spawn(async move {
            let mut pending: Vec<PathBuf> = Vec::new();

            loop {
                tokio::select! {
                    Some(path) = debounce_rx.recv() => {
                        if !pending.contains(&path) {
                            pending.push(path);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(300)), if !pending.is_empty() => {
                        let batch = std::mem::take(&mut pending);
                        let indexer_guard = indexer.read().await;
                        let Some(indexer) = indexer_guard.as_ref().cloned() else { continue };
                        let Some(repo) = repo.get().cloned() else { continue };

                        for path in batch {
                            let indexer = indexer.clone();
                            let path_clone = path.clone();
                            let result = tokio::task::spawn_blocking(move || indexer.index_file(&path_clone)).await;

                            // TODO: similar logic to did_save. maybe extract common logic?
                            match result {
                                Ok(Ok(Some((symbols, supers)))) => {
                                    for chunk in symbols.chunks(1000) {
                                        if let Err(e) = repo.insert_symbols(chunk).await {
                                            warn!("Failed to insert symbols: {e}");
                                        }
                                    }
                                    for chunk in supers.chunks(1000) {
                                        let mappings = chunk.iter()
                                            .map(|m| (&*m.symbol_fqn, &*m.super_short_name, m.super_fqn.as_deref()))
                                            .collect::<Vec<_>>();
                                        if let Err(e) = repo.insert_symbol_super_mappings(mappings).await {
                                            warn!("Failed to insert mappings: {e}");
                                        }
                                    }
                                    debug!("Re-indexed: {}", path.display());
                                }
                                Ok(Ok(None)) => warn!("Unsupported file type: {}", path.display()),
                                Ok(Err(e)) => warn!("Parse error, skipping: {e}"),
                                Err(e) => warn!("Failed to spawn index task: {e}"),
                            }
                        }
                    }
                }
            }
        });
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_fqn(
        &self,
        name: &str,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Option<String> {
        if name.contains('.') {
            return Some(name.to_string());
        }

        // Direct import match
        if let Some(import) = imports
            .iter()
            .find(|i| i.split('.').next_back() == Some(name))
        {
            return Some(import.clone());
        }

        // Wildcard import match
        for import in imports.iter().filter(|i| i.ends_with(".*")) {
            let tmp_fqn = import.replace("*", name);
            if (self
                .repo
                .get()
                .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)
                .ok()?
                .find_symbol_by_fqn(&tmp_fqn)
                .await
                .ok()?)
            .is_some()
            {
                return Some(tmp_fqn);
            }
            if let Ok(Some(_)) = self
                .repo
                .get()
                .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)
                .ok()?
                .find_external_symbol_by_fqn(&tmp_fqn)
                .await
            {
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

        if let Ok(Some(_)) = self
            .repo
            .get()
            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)
            .ok()?
            .find_external_symbol_by_fqn(&fallback_fqn)
            .await
        {
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
    ) -> Vec<ResolvedSymbol> {
        let class_fqn = match self
            .resolve_fqn(qualifier, imports.to_vec(), package_name.clone())
            .await
        {
            Some(fqn) => fqn,
            None => return vec![],
        };

        let mut visited = HashSet::new();
        self.try_members_with_inheritance(
            &class_fqn,
            member,
            &mut visited,
            imports.to_vec(),
            package_name,
        )
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn try_property_access(&self, class_fqn: &str, ident: &str) -> Option<Symbol> {
        // Try getter
        let getter_fqn = format!("{}#get{}", class_fqn, capitalize(ident));
        if let Ok(Some(found)) = self
            .repo
            .get()
            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)
            .ok()?
            .find_symbol_by_fqn(&getter_fqn)
            .await
        {
            return Some(found);
        }

        // Try boolean getter (isX for boolean properties)
        let is_getter_fqn = format!("{}#is{}", class_fqn, capitalize(ident));
        self.repo
            .get()
            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)
            .ok()?
            .find_symbol_by_fqn(&is_getter_fqn)
            .await
            .ok()
            .flatten()
    }

    async fn try_parent_member(
        &self,
        type_fqn: &str,
        member: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
        let type_symbol = match self.repo.get() {
            None => return vec![],
            Some(repo) => match repo.find_symbol_by_fqn(type_fqn).await {
                Ok(symbols) => symbols.into_iter().next(),
                Err(_) => None,
            },
        };

        let type_symbol = match type_symbol {
            Some(s) => s,
            None => return vec![],
        };

        let supers = match self.repo.get() {
            None => return vec![],
            Some(repo) => match repo
                .find_supers_by_symbol_fqn(&type_symbol.fully_qualified_name)
                .await
            {
                Ok(symbols) => symbols,
                Err(_) => return vec![],
            },
        };

        for super_name in supers.iter().map(|symbol| &symbol.fully_qualified_name) {
            let results = self
                .recurse_try_members_with_inheritance(
                    super_name,
                    member,
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
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
        if !visited.insert(type_fqn.to_string()) {
            return vec![];
        }

        let member_fqn = format!("{}#{}", type_fqn, member);

        // Try direct member
        if let Some(repo) = self.repo.get()
            && let Ok(found) = repo.find_symbols_by_fqn(&member_fqn).await
            && !found.is_empty()
        {
            return found.into_iter().map(ResolvedSymbol::Project).collect();
        }

        if let Some(found) = self.try_property_access(type_fqn, member).await {
            return vec![ResolvedSymbol::Project(found)];
        }

        let result = self
            .try_parent_member(type_fqn, member, visited, imports, package_name)
            .await;
        if !result.is_empty() {
            return result;
        }

        if let Some(repo) = self.repo.get()
            && let Ok(Some(found)) = repo.find_external_symbol_by_fqn(&member_fqn).await
        {
            return vec![ResolvedSymbol::External(found)];
        }

        vec![]
    }

    #[tracing::instrument(skip(self))]
    async fn recurse_try_members_with_inheritance(
        &self,
        parent_short_name: &str,
        member: &str,
        visited: &mut HashSet<String>,
        imports: Vec<String>,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
        tracing::info!("recurse_try_members_with_inheritance");
        let fqn = match self
            .resolve_fqn(parent_short_name, imports.clone(), package_name.clone())
            .await
        {
            Some(fqn) => fqn,
            None => return vec![],
        };

        let parent_symbol = match self.repo.get() {
            None => return vec![],
            Some(repo) => match repo.find_symbols_by_fqn(&fqn).await {
                Ok(symbols) => symbols.into_iter().next(),
                Err(_) => return vec![],
            },
        };

        match parent_symbol {
            Some(parent) => {
                Box::pin(self.try_members_with_inheritance(
                    &parent.fully_qualified_name,
                    member,
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

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all)]
    async fn resolve_type_member_chain(
        &self,
        qualifier: &str,
        member: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
        if let Some(current_type_fqn) = self
            .walk_member_chain(
                qualifier,
                lang,
                tree,
                content,
                imports.clone(),
                position,
                package_name,
            )
            .await
        {
            // Returns all overloads
            self.try_type_member(&current_type_fqn, member, &imports, None)
                .await
        } else {
            vec![]
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn walk_member_chain(
        &self,
        qualifier: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
    ) -> Option<String> {
        let parts: Vec<&str> = qualifier.split('#').collect();
        if parts.is_empty() {
            return None;
        }
        let base_type =
            if let Some(var_type) = lang.find_variable_type(tree, content, parts[0], position) {
                var_type
            } else {
                parts[0].to_string()
            };
        let mut current_type_fqn = match self
            .resolve_fqn(&base_type, imports.clone(), package_name.clone())
            .await
        {
            Some(fqn) => fqn,
            None => return None,
        };
        if parts.len() > 1 {
            for part in &parts[1..] {
                let symbols = self
                    .try_type_member(&current_type_fqn, part, &imports, None)
                    .await;
                let resolved = match symbols.into_iter().next() {
                    Some(s) => s,
                    None => return None,
                };

                current_type_fqn = if let Some(return_type) =
                    resolved.metadata().and_then(|m| m.return_type.as_ref())
                {
                    // For methods/fields, resolve their return/field type
                    let parent_package = resolved.package_name().unwrap_or_default().to_string();
                    match self
                        .resolve_fqn(return_type, imports.clone(), Some(parent_package))
                        .await
                    {
                        Some(fqn) => fqn,
                        None => return None,
                    }
                } else {
                    // For types (Class/Interface/Enum), use their FQN directly
                    resolved.package_name().unwrap_or_default().to_string()
                };
            }
        }

        Some(current_type_fqn)
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_type_member_chain(
        &self,
        qualifier: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
    ) -> Vec<ResolvedSymbol> {
        let Some(fqn) = self
            .walk_member_chain(
                qualifier,
                lang,
                tree,
                content,
                imports,
                position,
                package_name,
            )
            .await
        else {
            return vec![];
        };

        if let Some(repo) = self.repo.get() {
            if let Ok(symbols) = repo.find_symbols_by_parent_name(&fqn).await
                && !symbols.is_empty()
            {
                return symbols.into_iter().map(ResolvedSymbol::Project).collect();
            }
            repo.find_external_symbols_by_parent_name(&fqn)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(ResolvedSymbol::External)
                .collect()
        } else {
            vec![]
        }
    }

    async fn complete_by_prefix(&self, prefix: &str) -> Vec<ResolvedSymbol> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };

        if let Ok(symbols) = repo.find_symbols_by_prefix(prefix).await
            && !symbols.is_empty()
        {
            return symbols.into_iter().map(ResolvedSymbol::Project).collect();
        }
        repo.find_external_symbols_by_prefix(prefix)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(ResolvedSymbol::External)
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    async fn select_best_overload(
        &self,
        symbols: Vec<ResolvedSymbol>,
        call_args: Vec<(String, Position)>,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: &[String],
        package_name: Option<String>,
    ) -> Option<ResolvedSymbol> {
        let arg_count = call_args.len();

        let arity_matches: Vec<ResolvedSymbol> = symbols
            .into_iter()
            .filter(|s| {
                s.metadata()
                    .and_then(|m| m.parameters.as_ref())
                    .is_some_and(|params| params.len() == arg_count)
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
                if let Some(literal_type) = lang.get_literal_type(tree, content, position) {
                    literal_type
                } else {
                    lang.find_variable_type(tree, content, arg, position)
                        .unwrap_or_else(|| arg.clone())
                };

            let arg_fqn = self
                .resolve_fqn(&arg_type, imports.to_vec(), package_name.clone())
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
                            .resolve_fqn(&param_type, imports.to_vec(), Some(pkg_name.to_string()))
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
                    .is_some_and(|params| params.len() == expected_param_count),
                ResolvedSymbol::External(external) => external
                    .metadata
                    .parameters
                    .as_ref()
                    .is_some_and(|params| params.len() == expected_param_count),
                ResolvedSymbol::Local { .. } => false,
            })
            .collect()
    }

    async fn resolve_symbol_at_position(
        &self,
        params: &TextDocumentPositionParams,
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
                .resolve_fqn(&type_name, imports, package_name)
                .await
                .ok_or_else(|| {
                    tower_lsp::jsonrpc::Error::invalid_params("Failed to find FQN by location")
                })?;

            return self.fqn_to_symbols(fqn).await;
        }

        if let Some((ident, qualifier)) = lang.find_ident_at_position(&tree, &content, &position) {
            match qualifier {
                Some(q) => {
                    let symbols = self
                        .resolve_type_member_chain(
                            &q,
                            &ident,
                            lang,
                            &tree,
                            &content,
                            imports.clone(),
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

                    if let Some(args) = lang.extract_call_arguments(&tree, &content, &position)
                        && let Some(symbol) = self
                            .select_best_overload(
                                symbols.clone(),
                                args,
                                lang,
                                &tree,
                                &content,
                                &imports,
                                package_name,
                            )
                            .await
                    {
                        return Ok(vec![symbol]);
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
                        .resolve_fqn(&ident, imports, package_name)
                        .await
                        .ok_or_else(|| {
                            tower_lsp::jsonrpc::Error::invalid_params(
                                "Failed to find FQN by location",
                            )
                        })?;

                    self.fqn_to_symbols(fqn).await
                }
            }
        } else {
            Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Failed to get ident/type name",
            ))
        }
    }

    #[tracing::instrument(skip_all)]
    async fn fqn_to_symbols(&self, fqn: String) -> Result<Vec<ResolvedSymbol>> {
        let repo = self
            .repo
            .get()
            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)?;

        if let Ok(Some(symbol)) = repo.find_symbol_by_fqn(&fqn).await {
            return Ok(vec![ResolvedSymbol::Project(symbol)]);
        }
        let external_symbol = repo
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
            p.components()
                .any(|c| matches!(c.as_os_str().to_str(), Some(".gradle" | ".m2" | "caches")))
        });

        false
    }

    fn get_line_at(&self, pos: &TextDocumentPositionParams) -> Option<String> {
        let uri = pos.text_document.uri.to_string();
        let ttl = Duration::from_secs(FILE_CACHE_TTL_SECS);

        if let Some(entry) = self.documents.get(&uri)
            && entry.1.elapsed() < ttl
        {
            return entry
                .0
                .lines()
                .nth(pos.position.line as usize)
                .map(str::to_string);
        }

        let path = pos.text_document.uri.to_file_path().ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        let line = text
            .lines()
            .nth(pos.position.line as usize)
            .map(str::to_string);
        self.documents.insert(uri, (text, Instant::now()));
        line
    }

    async fn handle_build_file_changed(&self, root: &Path) {
        let manifest_path = root.join(MANIFEST_PATH_FRAGMENT);

        let previous: Vec<(Option<PathBuf>, Option<PathBuf>)> = tokio::fs::read(&manifest_path)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let build_tool_guard = self.build_tool.read().await;
        let Some(build_tool) = build_tool_guard.as_ref().cloned() else {
            return;
        };
        drop(build_tool_guard);

        let root_clone = root.to_path_buf();
        let Ok(Ok(current)) =
            tokio::task::spawn_blocking(move || build_tool.get_dependency_paths(&root_clone)).await
        else {
            lsp_error!("Failed to resolve dependencies");
            return;
        };

        let previous_jars: HashSet<PathBuf> =
            previous.iter().filter_map(|(b, _)| b.clone()).collect();
        let current_jars: HashSet<PathBuf> =
            current.iter().filter_map(|(b, _)| b.clone()).collect();

        let removed: Vec<PathBuf> = previous_jars.difference(&current_jars).cloned().collect();
        let added: Vec<(Option<PathBuf>, Option<PathBuf>)> = current
            .iter()
            .filter(|(b, _)| b.as_ref().map_or(false, |p| !previous_jars.contains(p)))
            .cloned()
            .collect();

        let Some(repo) = self.repo.get().cloned() else {
            return;
        };

        for jar in &removed {
            if let Err(e) = repo
                .delete_external_symbols_for_jar(&jar.to_string_lossy())
                .await
            {
                lsp_error!("Failed to remove stale JAR {}: {e}", jar.display());
            }
        }

        if !added.is_empty() {
            let indexer_guard = self.indexer.read().await;
            let Some(indexer) = indexer_guard.as_ref().cloned() else {
                return;
            };
            drop(indexer_guard);
            indexer
                .index_external_deps(added, |_, _| {}, |_, _| {})
                .await;
        }

        if let Ok(json) = serde_json::to_string(&current) {
            if let Err(e) = tokio::fs::write(&manifest_path, json).await {
                lsp_error!("Failed to update manifest file: {e}");
            }
        }
    }

    fn needs_full_reindex(&self, root: &Path) -> bool {
        let version_path = root.join(INDEX_PATH_FRAGMENT);
        let db_path = root.join(DB_PATH_FRAGMENT);
        let manifest_path = root.join(MANIFEST_PATH_FRAGMENT);

        if !manifest_path.exists() || !db_path.exists() {
            return true;
        }

        match std::fs::read_to_string(&version_path) {
            Ok(v) => v.trim() != INDEX_VERSION,
            Err(_) => true,
        }
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

            // test setup initialized the repo before this stage
            if self.repo.get().is_none() {
                let (dir_fragment, file_name) = DB_PATH_FRAGMENT
                    .split_once('/')
                    .expect(&format!("Failed to split {DB_PATH_FRAGMENT} directory"));

                let lspintar_dir = root.join(dir_fragment);
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(&lspintar_dir)
                    .map_err(|e| {
                        tracing::error!("failed to create {dir_fragment} dir: {}", e);
                        tower_lsp::jsonrpc::Error::internal_error()
                    })?;

                let db_path = lspintar_dir.join(file_name);
                let repo = Repository::new(db_path.to_str().unwrap())
                    .await
                    .map_err(|e| {
                        debug!("Failed to create {DB_PATH_FRAGMENT} in {:?}: {e}", root);
                        tower_lsp::jsonrpc::Error::internal_error()
                    })?;

                self.repo.set(Arc::new(repo)).ok();
            }

            *self.workspace_root.write().await = Some(root);
        } else {
            debug!("workspace root not found, shutting down");
            std::process::exit(0);
        }

        let documents = self.documents.clone();
        tokio::spawn(async move {
            let ttl = Duration::from_secs(FILE_CACHE_TTL_SECS);
            let interval = Duration::from_secs(FILE_CACHE_TTL_SECS * 2);
            loop {
                tokio::time::sleep(interval).await;
                documents.retain(|_, (_, instant)| instant.elapsed() < ttl);
            }
        });

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
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(
                        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ."
                            .chars()
                            .map(|c| c.to_string())
                            .collect(),
                    ),
                    ..Default::default()
                }),
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
            let Some(repo) = self.repo.get() else {
                lsp_error!("Failed to initialize index repository");
                return;
            };

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
            *self.build_tool.write().await = Some(Arc::clone(&build_tool));

            let mut indexer = Indexer::new(Arc::clone(repo));
            languages.iter().for_each(|(k, v)| {
                indexer.register_language(k, v.clone());
            });

            if self.needs_full_reindex(&root) {
                debug!("Full reindex required, clearing existing index.");
                let _ = tokio::fs::remove_file(root.join(MANIFEST_PATH_FRAGMENT)).await;
                if let Err(e) = repo.clear_all().await {
                    lsp_error!("Failed to clear index: {e}");
                    return;
                }

                let indexing_start = Instant::now();

                lsp_info!("Resolving dependencies...");
                tokio::time::sleep(Duration::from_millis(500)).await;

                let external_deps = match build_tool.get_dependency_paths(&root) {
                    Ok(deps) => deps,
                    Err(e) => {
                        let message = format!("Failed to get dependencies: {e}");
                        lsp_error!("{}", message);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        panic!("{}", message);
                    }
                };
                let jdk_sources = match build_tool.get_jdk_dependency_path(&root) {
                    Ok(deps) => deps,
                    Err(e) => {
                        let message = format!("Failed to get JDK sources: {e}");
                        lsp_error!("{}", message);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        panic!("{}", message);
                    }
                };
                let mut jars: Vec<(Option<PathBuf>, Option<PathBuf>)> = external_deps;

                // exclude JDK
                let jars_for_manifest = jars.clone();

                if let Some(src_zip) = jdk_sources {
                    jars.push((None, Some(src_zip)));
                }

                let token_ws = format!("idx-ws-{}", uuid::Uuid::new_v4());
                let token_ws_end = token_ws.clone();

                let token_ws_save = format!("idx-ws-save-{}", uuid::Uuid::new_v4());
                let token_ws_save_end = token_ws_save.clone();

                lsp_progress_begin!(&token_ws, "Indexing...");

                let save_ws_begun = std::sync::Once::new();

                let ws_result = indexer
                    .index_workspace(
                        &root,
                        move |completed, total| {
                            lsp_progress!(
                                &token_ws,
                                &format!("(1/2) Indexing workspace ({}/{})", completed, total),
                                (completed as f32 / total as f32) * 100.0
                            );
                            if completed == total {
                                lsp_progress_end!(&token_ws_end);
                            }
                        },
                        move |completed, total| {
                            save_ws_begun.call_once(|| {
                                lsp_progress_begin!(&token_ws_save, "Saving data...")
                            });
                            lsp_progress!(
                                &token_ws_save,
                                &format!(
                                    "(2/2) Saving project symbol indexes ({}/{})",
                                    completed, total
                                ),
                                (completed as f32 / total as f32) * 100.0
                            );
                            if completed == total {
                                lsp_progress_end!(&token_ws_save_end);
                            }
                        },
                    )
                    .await;

                if let Err(e) = ws_result {
                    let message = format!("Failed to index workspace: {e}");
                    lsp_error!("{}", message);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    panic!("{}", message);
                }

                let token_jar = format!("idx-ext-{}", uuid::Uuid::new_v4());
                let token_jar_end = token_jar.clone();

                let token_jar_save = format!("idx-ext-save-{}", uuid::Uuid::new_v4());
                let token_jar_save_end = token_jar_save.clone();

                lsp_progress_begin!(&token_jar, "Indexing...");

                let save_jar_begun = std::sync::Once::new();

                indexer
                    .index_external_deps(
                        jars,
                        move |completed, total| {
                            lsp_progress!(
                                &token_jar,
                                &format!("(2/2) Indexing JARs ({}/{})", completed, total),
                                (completed as f32 / total as f32) * 100.0
                            );
                            if completed == total {
                                lsp_progress_end!(&token_jar_end);
                            }
                        },
                        move |completed, total| {
                            save_jar_begun.call_once(|| {
                                lsp_progress_begin!(&token_jar_save, "Saving data...")
                            });
                            lsp_progress!(
                                &token_jar_save,
                                &format!(
                                    "(2/2) Saving external symbol indexes ({}/{})",
                                    completed, total
                                ),
                                (completed as f32 / total as f32) * 100.0
                            );
                            if completed == total {
                                lsp_progress_end!(&token_jar_save_end);
                            }
                        },
                    )
                    .await;

                let manifest_path = root.join(MANIFEST_PATH_FRAGMENT);
                match serde_json::to_string(&jars_for_manifest) {
                    Ok(json) => {
                        if let Err(e) = tokio::fs::write(&manifest_path, json).await {
                            lsp_error!("Failed to write manifest file: {e}");
                        }
                    }
                    Err(e) => lsp_error!("Failed to serialize manifest file: {e}"),
                }

                lsp_info!(
                    "Indexing finished in {:.2}s",
                    indexing_start.elapsed().as_secs_f64()
                );
            }

            *indexer_lock.write().await = Some(indexer);
            *vcs_handler_lock.write().await = Some(vcs);
            *workspace_root_lock.write().await = Some(root.clone());

            if let Some(vcs) = self.vcs_handler.read().await.as_ref() {
                if let Ok(rev) = vcs.get_current_revision() {
                    *self.last_known_revision.write().await = Some(rev);
                }
            }

            if let Err(e) = tokio::fs::write(root.join(INDEX_PATH_FRAGMENT), INDEX_VERSION).await {
                lsp_error!("Failed to write {INDEX_PATH_FRAGMENT}: {e}");
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let symbols = self
            .resolve_symbol_at_position(&params.text_document_position_params)
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
                    "Failed to get language support".to_string(),
                )
            })?;

            let (tree, content) = lang.parse(&path).ok_or_else(|| {
                tower_lsp::jsonrpc::Error::invalid_params("Failed to parse file".to_string())
            })?;

            let imports = lang.get_imports(&tree, &content);
            let package_name = lang.get_package_name(&tree, &content);

            let position = params.text_document_position_params.position;

            if let Some((ident, _)) = lang.find_ident_at_position(&tree, &content, &position) {
                if let Some(type_name) =
                    lang.get_type_at_position(tree.root_node(), &content, &position)
                {
                    let fqn = self
                        .resolve_fqn(&type_name, imports, package_name)
                        .await
                        .ok_or(tower_lsp::jsonrpc::Error::invalid_params(
                            "Failed to find FQN by location".to_string(),
                        ))?;

                    let implementations = self
                        .repo
                        .get()
                        .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)?
                        .find_super_impls_by_fqn(&fqn)
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
                            .get()
                            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)?
                            .find_super_impls_by_short_name(&type_name)
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
                            .map(ResolvedSymbol::Project)
                            .collect(),
                    ));
                };

                if let Some((receiver_type, params)) =
                    lang.get_method_receiver_and_params(tree.root_node(), &content, &position)
                {
                    let parent_fqn = self
                        .resolve_fqn(&receiver_type, imports, package_name)
                        .await
                        .ok_or_else(|| {
                            tower_lsp::jsonrpc::Error::invalid_params("Failed to resolve FQN")
                        })?;

                    let implementations = self
                        .repo
                        .get()
                        .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)?
                        .find_super_impls_by_fqn(&parent_fqn)
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
                            .get()
                            .ok_or_else(tower_lsp::jsonrpc::Error::internal_error)?
                            .find_symbols_by_fqn(&method_fqn)
                            .await
                        {
                            let resolved: Vec<ResolvedSymbol> =
                                symbols.into_iter().map(ResolvedSymbol::Project).collect();

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
        let symbols = self
            .resolve_symbol_at_position(&params.text_document_position_params)
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

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            let uri = params.text_document.uri.to_string();
            self.documents.insert(uri, (change.text, Instant::now()));
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let path = match params.text_document.uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };
        let Some(indexer) = self.indexer.read().await.as_ref().cloned() else {
            return;
        };
        let Some(repo) = self.repo.get().cloned() else {
            return;
        };

        let path_clone = path.clone();
        let result = tokio::task::spawn_blocking(move || indexer.index_file(&path_clone)).await;

        match result {
            Ok(Ok(Some((symbols, supers)))) => {
                for chunk in symbols.chunks(1000) {
                    if let Err(e) = repo.insert_symbols(chunk).await {
                        warn!("Failed to insert symbols on save: {e}");
                    }
                }
                for chunk in supers.chunks(1000) {
                    let mappings = chunk
                        .iter()
                        .map(|m| (&*m.symbol_fqn, &*m.super_short_name, m.super_fqn.as_deref()))
                        .collect::<Vec<_>>();
                    if let Err(e) = repo.insert_symbol_super_mappings(mappings).await {
                        warn!("Failed to insert mappings on save: {e}");
                    }
                }
                debug!("Re-indexed: {}", path.display());
            }
            Ok(Ok(None)) => warn!("Unsupported file type, ignore"),
            Ok(Err(e)) => warn!("Parse error on save, skipping reindex: {e}"),
            Err(e) => warn!("Failed to spawn index task: {e}"),
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let pos = &params.text_document_position;

        let line = self
            .get_line_at(pos)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Cannot read file"))?;
        let char_pos = pos.position.character as usize;

        let path = PathBuf::from_str(pos.text_document.uri.path()).unwrap();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("No file extension"))?;
        let lang = self
            .languages
            .get(ext)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Unsupported language"))?;
        let (tree, content) = lang
            .parse(&path)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Failed to parse file"))?;
        let imports = lang.get_imports(&tree, &content);
        let package_name = lang.get_package_name(&tree, &content);

        let line_prefix = if line.is_empty() || char_pos == 0 {
            ""
        } else {
            line.char_indices()
                .nth(char_pos)
                .map(|(i, _)| &line[..i])
                .unwrap_or(&line)
        };
        let symbols = if line_prefix.contains('.') {
            let receiver = extract_receiver(&line, char_pos).unwrap_or("");
            self.complete_type_member_chain(
                receiver,
                lang,
                &tree,
                &content,
                imports.clone(),
                &pos.position,
                package_name.clone(),
            )
            .await
        } else {
            let prefix = extract_prefix(&line, char_pos);
            let mut symbols = self.complete_by_prefix(prefix).await;

            let scope_decls = lang.find_declarations_in_scope(&tree, &content, &pos.position);
            for (name, var_type) in scope_decls {
                if name.starts_with(prefix) {
                    symbols.push(ResolvedSymbol::Local {
                        uri: params.text_document_position.text_document.uri.clone(),
                        position: pos.position,
                        name,
                        var_type,
                    });
                }
            }
            symbols
        };

        let items: Vec<CompletionItem> =
            symbols
                .into_iter()
                .map(|s| match s {
                    ResolvedSymbol::External(_) | ResolvedSymbol::Project(_) => CompletionItem {
                        label: s.name().to_string(),
                        kind: s.node_kind().to_lsp_kind(),
                        detail: Some(s.package_name().unwrap_or_default().to_string()),
                        additional_text_edits: if lang.get_implicit_imports().iter().any(|i| {
                            i.trim_end_matches(".*") == s.package_name().unwrap_or_default()
                        }) {
                            None
                        } else {
                            match s {
                                ResolvedSymbol::External(ext) => {
                                    let import_fqn = ext
                                        .fully_qualified_name
                                        .split('#')
                                        .next()
                                        .unwrap_or(&ext.fully_qualified_name);

                                    if !imports.contains(&import_fqn.to_string()) {
                                        let import_text_edit = get_import_text_edit(
                                            &content,
                                            &ext.fully_qualified_name,
                                            &ext.package_name,
                                            &ext.parent_name.unwrap_or_default(),
                                            lang.get_language(),
                                        );
                                        Some(vec![import_text_edit])
                                    } else {
                                        None
                                    }
                                }

                                ResolvedSymbol::Project(sym) => {
                                    let import_fqn = sym
                                        .fully_qualified_name
                                        .split('#')
                                        .next()
                                        .unwrap_or(&sym.fully_qualified_name);

                                    if !imports.contains(&import_fqn.to_string())
                                        && sym.package_name
                                            != package_name.as_deref().unwrap_or_default()
                                    {
                                        let import_text_edit = get_import_text_edit(
                                            &content,
                                            &sym.fully_qualified_name,
                                            &sym.package_name,
                                            &sym.parent_name.unwrap_or_default(),
                                            lang.get_language(),
                                        );
                                        Some(vec![import_text_edit])
                                    } else {
                                        None
                                    }
                                }
                                ResolvedSymbol::Local { .. } => None,
                            }
                        },
                        ..Default::default()
                    },
                    ResolvedSymbol::Local { name, var_type, .. } => CompletionItem {
                        label: name,
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: var_type,
                        ..Default::default()
                    },
                })
                .collect();

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let Some(root) = self.workspace_root.read().await.clone() else {
            return;
        };

        let vcs_guard = self.vcs_handler.read().await;
        let revision_file = vcs_guard
            .as_ref()
            .and_then(|vcs| vcs.get_revision_file(&root));

        for change in params.changes {
            let Ok(path) = change.uri.to_file_path() else {
                continue;
            };

            if change.typ == FileChangeType::DELETED {
                self.documents.remove(&change.uri.to_string());
                let Some(repo) = self.repo.get() else {
                    continue;
                };
                if let Err(e) = repo.delete_symbols_for_file(&path.to_string_lossy()).await {
                    lsp_error!("Failed to remove symbols for {}: {e}", path.display());
                }
            } else if revision_file.as_deref() == Some(&path) {
                let Some(vcs) = vcs_guard.as_ref() else {
                    continue;
                };
                let Ok(new_rev) = vcs.get_current_revision() else {
                    continue;
                };
                let old_rev = self.last_known_revision.read().await.clone();

                if let Some(old) = old_rev {
                    if old != new_rev {
                        if let Ok(changed) = vcs.get_changed_files(&old, &new_rev, &root) {
                            for p in changed {
                                let _ = self.debounce_tx.send(p).await;
                            }
                        }
                    }
                }

                *self.last_known_revision.write().await = Some(new_rev);
            } else {
                let build_tool_guard = self.build_tool.read().await;
                if let Some(build_tool) = build_tool_guard.as_ref() {
                    if build_tool.is_build_file(&path) {
                        drop(build_tool_guard);
                        self.handle_build_file_changed(&root).await;
                        continue;
                    }
                }

                let _ = self.debounce_tx.send(path).await;
            }
        }
    }
}
