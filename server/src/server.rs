use core::panic;
use dashmap::DashMap;
use futures::{StreamExt, stream};
use groovy::GroovySupport;
use java::JavaSupport;
use kotlin::KotlinSupport;
use lsp_core::{
    build_tools::{BuildToolHandler, SubprojectClasspath, get_build_tool},
    language_support::LanguageSupport,
    languages::Language,
    lsp_error, lsp_info, lsp_logging, lsp_progress, lsp_progress_begin, lsp_progress_end,
    util::{capitalize, extract_prefix, extract_receiver, get_import_text_edit},
    vcs::{VcsHandler, get_vcs_handler},
};
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
        APP_VERSION, CLASSPATH_MANIFEST_PATH_FRAGMENT, DB_PATH_FRAGMENT, FILE_CACHE_TTL_SECS,
        INDEX_PATH_FRAGMENT, MANIFEST_PATH_FRAGMENT, VCS_REVISION_PATH_FRAGMENT,
    },
    enums::ResolvedSymbol,
    generic_resolution::{build_type_bindings, parse_type_ref, substitute_type_vars},
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
    pub(crate) languages: HashMap<String, Arc<dyn LanguageSupport + Send + Sync>>,
    vcs_handler: Arc<RwLock<Option<Arc<dyn VcsHandler + Send + Sync>>>>,
    last_known_revision: Arc<RwLock<Option<String>>>,
    build_tool: Arc<RwLock<Option<Arc<dyn BuildToolHandler + Send + Sync>>>>,

    // Optimizations
    /// Caches open document contents to avoid excessive I/O reads.
    pub documents: DashMap<String, (String, Instant)>,
    /// Debounces `didChangeWatchedFiles` to avoid redundant reindexing.
    debounce_tx: tokio::sync::mpsc::Sender<PathBuf>,
    /// Debounces `textDocument/didChange` to trigger diagnostics after 300 ms of idle.
    diag_debounce_tx: tokio::sync::mpsc::Sender<Url>,

    /// Per-sub-project source-root → classpath JAR mapping.
    /// Empty when the workspace is a single-project build.
    subproject_classpath: Arc<RwLock<Vec<SubprojectClasspath>>>,

    /// Set to true once the initial indexing pass completes. Diagnostics that rely on
    /// cross-file symbol lookups are suppressed while this is false to avoid bogus errors
    /// from a half-populated index.
    index_ready: Arc<AtomicBool>,
}

/// Java primitive types and keywords that are never unresolved.
const TYPE_REF_SKIP_LIST: &[&str] = &[
    "boolean", "byte", "char", "double", "float", "int", "long", "short", "void", "var",
];

/// Methods universally available on every Java/Kotlin/Groovy object (java.lang.Object).
/// Included in the reachable-method set to avoid false-positive method_not_found diagnostics.
const JAVA_OBJECT_METHODS: &[&str] = &[
    "equals", "hashCode", "toString", "getClass", "clone", "finalize",
    "notify", "notifyAll", "wait",
];

/// Numeric primitive width used for narrowing_conversion detection.
/// Returns `None` for non-numeric or non-primitive types.
fn numeric_width(t: &str) -> Option<u8> {
    match t {
        "byte" => Some(1),
        "short" => Some(2),
        "int" => Some(3),
        "long" => Some(4),
        "float" => Some(5),
        "double" => Some(6),
        _ => None,
    }
}

/// Returns true when assigning `rhs_type` to a variable of `lhs_type` is a narrowing conversion.
fn is_narrowing_conversion(lhs_type: &str, rhs_type: &str) -> bool {
    match (numeric_width(lhs_type), numeric_width(rhs_type)) {
        (Some(lw), Some(rw)) => rw > lw,
        _ => false,
    }
}

/// Strips generic type arguments and Kotlin nullable markers from a type name, returning
/// the bare base name for comparison purposes.  E.g. `"List<String>"` → `"List"`, `"Int?"` → `"Int"`.
fn strip_type_args(t: &str) -> &str {
    t.split('<').next().unwrap_or(t).trim_end_matches('?').trim()
}

/// Returns true when a return-type base name is too broad to make a meaningful comparison.
fn is_unconstrained_return_type(t: &str) -> bool {
    matches!(t, "void" | "Unit" | "Object" | "Any" | "V" | "T" | "R" | "E")
}

/// Returns true when a type reference should be skipped during unresolved-symbol checking:
///   - Java primitive / keyword types
///   - Types declared in the same file
///   - Single-character names (likely generic type parameters such as T, E, K, V)
fn is_type_ref_skippable(name: &str, local_types: &[String]) -> bool {
    TYPE_REF_SKIP_LIST.contains(&name)
        || local_types.iter().any(|t| t == name)
        || (name.len() == 1 && name.chars().next().is_some_and(|c| c.is_uppercase()))
}

/// Returns true if `(line, col)` is inside a comment node in the parse tree.
/// Works for any language because all tree-sitter comment node kinds contain "comment".
fn position_in_comment(tree: &tree_sitter::Tree, line: usize, col: usize) -> bool {
    let point = tree_sitter::Point::new(line, col);
    let Some(mut node) = tree.root_node().descendant_for_point_range(point, point) else {
        return false;
    };
    loop {
        if node.kind().contains("comment") {
            return true;
        }
        match node.parent() {
            Some(p) => node = p,
            None => return false,
        }
    }
}

/// Maps a literal AST node kind (+ its text) to a base type name for argument-type comparison.
/// Returns `None` when the argument is not a simple literal (complex expressions are skipped).
fn arg_literal_base_type<'a>(node_kind: &'a str, text: &str) -> Option<&'a str> {
    match node_kind {
        // Java/Groovy/Kotlin integer literals
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "hex_literal"
        | "bin_literal" => {
            if text.ends_with('l') || text.ends_with('L') {
                Some("long")
            } else {
                Some("int")
            }
        }
        // Java/Groovy float literals
        "decimal_floating_point_literal" | "hex_floating_point_literal" => {
            if text.to_lowercase().ends_with('f') { Some("float") } else { Some("double") }
        }
        // Kotlin float literal
        "real_literal" => {
            if text.ends_with('f') || text.ends_with('F') { Some("float") } else { Some("double") }
        }
        "true" | "false" | "boolean_literal" => Some("boolean"),
        "string_literal" | "text_block" | "multiline_string_literal" => Some("String"),
        "character_literal" => Some("char"),
        "null_literal" | "null" => Some("null"),
        _ => None,
    }
}

/// Returns true when a literal of type `arg_base` is compatible with a declared parameter type.
/// Conservative: returns `true` (compatible) when unsure.
fn is_arg_compatible_with_param(arg_base: &str, param_type: &str) -> bool {
    if arg_base == "null" {
        // null is compatible with any reference type (not with non-null primitives)
        return !matches!(param_type, "int" | "long" | "short" | "byte" | "char" | "float" | "double" | "boolean");
    }
    // Strip package prefix and generics for comparison
    let p = param_type.split('.').next_back().unwrap_or(param_type);
    let p = p.split('<').next().unwrap_or(p).trim_end_matches('?').trim();
    match arg_base {
        "String" => matches!(p, "String" | "CharSequence" | "Object" | "Any" | "Serializable" | "Comparable"),
        "int" => matches!(p, "int" | "Integer" | "long" | "Long" | "float" | "Float" | "double" | "Double" | "short" | "Short" | "byte" | "Byte" | "Number" | "Object" | "Any" | "Comparable"),
        "long" => matches!(p, "long" | "Long" | "float" | "Float" | "double" | "Double" | "Number" | "Object" | "Any"),
        "float" => matches!(p, "float" | "Float" | "double" | "Double" | "Number" | "Object" | "Any"),
        "double" => matches!(p, "double" | "Double" | "Number" | "Object" | "Any"),
        "boolean" => matches!(p, "boolean" | "Boolean" | "Object" | "Any"),
        "char" => matches!(p, "char" | "Character" | "int" | "Integer" | "long" | "Long" | "Object" | "Any"),
        _ => true, // unknown arg type — don't flag
    }
}

/// Returns a sort key for completion suggestions.
/// Lower values appear first:
///   0 – local variables / method parameters (most relevant)
///   1 – project symbols in the same package as the current file
///   2 – project symbols in a different package
///   3 – external (JAR) symbols
fn completion_rank(symbol: &ResolvedSymbol, current_package: Option<&str>) -> u8 {
    match symbol {
        ResolvedSymbol::Local { .. } => 0,
        ResolvedSymbol::Project(s) => {
            if current_package.is_some_and(|pkg| pkg == s.package_name) {
                1
            } else {
                2
            }
        }
        ResolvedSymbol::External(_) => 3,
    }
}

impl Backend {
    pub fn new(client: tower_lsp::Client) -> Self {
        lsp_logging::init_logging_service(client.clone());

        let mut languages: HashMap<String, Arc<dyn LanguageSupport + Send + Sync>> = HashMap::new();
        languages.insert("groovy".to_string(), Arc::new(GroovySupport::new()));
        languages.insert("java".to_string(), Arc::new(JavaSupport::new()));
        languages.insert("kt".to_string(), Arc::new(KotlinSupport::new()));

        let (debounce_tx, debounce_rx) = tokio::sync::mpsc::channel::<PathBuf>(64);
        let (diag_debounce_tx, diag_debounce_rx) = tokio::sync::mpsc::channel::<Url>(64);
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
            diag_debounce_tx,
            subproject_classpath: Arc::new(RwLock::new(vec![])),
            index_ready: Arc::new(AtomicBool::new(false)),
        };

        backend.spawn_debounce_task(debounce_rx);
        backend.spawn_diag_debounce_task(diag_debounce_rx);
        backend
    }

    fn spawn_debounce_task(&self, mut debounce_rx: tokio::sync::mpsc::Receiver<PathBuf>) {
        let indexer = Arc::clone(&self.indexer);
        let repo = self.repo.clone();
        let backend = self.clone();

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
                            let buffered = Url::from_file_path(&path)
                                .ok()
                                .and_then(|uri| backend.documents.get(&uri.to_string()).map(|e| e.0.clone()));
                            let result = tokio::task::spawn_blocking(move || match buffered {
                                Some(content) => indexer.index_content(&path_clone, &content),
                                None => indexer.index_file(&path_clone),
                            }).await;

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

                                    if let Ok(uri) = Url::from_file_path(&path) {
                                        backend.publish_diagnostics(uri).await;
                                    }
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

    fn spawn_diag_debounce_task(&self, mut rx: tokio::sync::mpsc::Receiver<Url>) {
        let backend = self.clone();
        tokio::spawn(async move {
            let mut pending: Vec<Url> = Vec::new();
            loop {
                tokio::select! {
                    Some(uri) = rx.recv() => {
                        if !pending.contains(&uri) {
                            pending.push(uri);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(300)), if !pending.is_empty() => {
                        for uri in std::mem::take(&mut pending) {
                            backend.publish_diagnostics(uri).await;
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

    /// Like `resolve_fqn` but returns `None` when the FQN is only a guess.
    ///
    /// Used exclusively for `unresolved_symbol` diagnostics where a false positive
    /// (flagging a valid type as unresolved) is worse than a false negative.
    ///
    /// Rules:
    /// - Already-qualified name (`foo.Bar`) → `Some` always (we trust it).
    /// - Direct explicit import (`import foo.Bar`) → `Some` always (we trust it).
    /// - Wildcard import (`import foo.*`) → `Some(foo.Bar)` only when verified in DB.
    /// - Same-package fallback → `Some(pkg.Bar)` only when verified in project DB.
    /// - Everything else → `None` (no emit, rather than false positive).
    async fn resolve_fqn_strict(
        &self,
        name: &str,
        imports: &[String],
        package_name: Option<String>,
    ) -> Option<String> {
        if name.contains('.') {
            return Some(name.to_string());
        }

        // Direct non-wildcard import — trust it; outer check will emit if absent from DB.
        if let Some(import) = imports
            .iter()
            .find(|i| !i.ends_with(".*") && i.split('.').next_back() == Some(name))
        {
            return Some(import.clone());
        }

        let repo = self.repo.get()?;

        // Wildcard import match — only return when DB-verified.
        for import in imports.iter().filter(|i| i.ends_with(".*")) {
            let tmp_fqn = import.replace("*", name);
            if repo.find_symbol_by_fqn(&tmp_fqn).await.ok().flatten().is_some() {
                return Some(tmp_fqn);
            }
            if let Ok(Some(_)) = repo.find_external_symbol_by_fqn(&tmp_fqn).await {
                return Some(tmp_fqn);
            }
        }

        // Same-package fallback — only return when found in project DB.
        if let Some(pkg) = package_name {
            let fallback = format!("{}.{}", pkg, name);
            if repo.find_symbol_by_fqn(&fallback).await.ok().flatten().is_some() {
                return Some(fallback);
            }
        }

        // Could not verify — suppress to avoid false positives.
        None
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
            tracing::info!("found: {:?}", found);
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

        let resolved_fqn = match self.repo.get() {
            None => return vec![],
            Some(repo) => {
                if let Ok(Some(s)) = repo.find_symbol_by_fqn(&fqn).await {
                    s.fully_qualified_name
                } else if let Ok(Some(s)) = repo.find_external_symbol_by_fqn(&fqn).await {
                    s.fully_qualified_name
                } else {
                    return vec![];
                }
            }
        };

        Box::pin(self.try_members_with_inheritance(
            &resolved_fqn,
            member,
            visited,
            imports,
            package_name,
        ))
        .await
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
        Box::pin(self.walk_member_chain_inner(
            qualifier,
            lang,
            tree,
            content,
            imports,
            position,
            package_name,
            &HashMap::new(),
        ))
        .await
    }

    /// Internal chain walker that supports an explicit scope override map.
    /// Variables named in `scope_overrides` are resolved to their overridden type
    /// instead of querying the AST — used to pass lambda parameter types when
    /// resolving lambda body chains for InferLambdaReturnType.
    #[allow(clippy::too_many_arguments)]
    async fn walk_member_chain_inner(
        &self,
        qualifier: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
        scope_overrides: &HashMap<String, String>,
    ) -> Option<String> {
        // Split off lambda body info: "items#map__lb__param|body_chain"
        let (chain_part, lambda_body_info) = if let Some(idx) = qualifier.find("__lb__") {
            (&qualifier[..idx], Some(&qualifier[idx + 6..]))
        } else {
            (qualifier, None)
        };

        let parts: Vec<&str> = chain_part.split('#').collect();
        if parts.is_empty() {
            return None;
        }

        // Resolve the base variable's type (may carry generic args like "List<String>").
        // find_variable_type may return two kinds of special strings:
        //   "Foo#bar"           — chain expression; resolve recursively.
        //   "__cp__:..."        — closure/lambda param; resolve from method signature.
        let base_type_str = {
            let raw = if let Some(overridden) = scope_overrides.get(parts[0]) {
                overridden.clone()
            } else {
                let vtype = lang.find_variable_type(tree, content, parts[0], position);
                tracing::debug!("[LSPINTAR_COMPLETION] find_variable_type({:?}) = {:?}", parts[0], vtype);
                vtype.unwrap_or_else(|| parts[0].to_string())
            };
            if raw.starts_with("__cp__:") {
                Box::pin(self.resolve_closure_param_type(
                    &raw,
                    lang,
                    tree,
                    content,
                    imports.clone(),
                    position,
                    package_name.clone(),
                ))
                .await
                .unwrap_or_else(|| "java.lang.Object".to_string())
            } else if raw.contains('#') {
                Box::pin(self.walk_member_chain_inner(
                    &raw,
                    lang,
                    tree,
                    content,
                    imports.clone(),
                    position,
                    package_name.clone(),
                    scope_overrides,
                ))
                .await
                .unwrap_or(raw)
            } else {
                raw
            }
        };

        // Split into name + receiver type args: "List<String>" → ("List", ["String"])
        let (base_name, mut current_type_args) = parse_type_ref(&base_type_str);

        let mut current_type_fqn = self
            .resolve_fqn(&base_name, imports.clone(), package_name.clone())
            .await?;

        let parts_len = parts.len();
        for (step_idx, part) in parts[1..].iter().enumerate() {
            let is_last_step = step_idx == parts_len - 2;

            // Parse optional call-site type args encoded in the step.
            // Format: "method__ca__TypeArg1,TypeArg2" encodes explicit call-site type arguments
            // (e.g. from list.<String>map(...) in Java/Groovy or list.map<String>(...) in Kotlin).
            let (method_name, call_site_type_args): (&str, Vec<String>) =
                if let Some(idx) = part.find("__ca__") {
                    let args = part[idx + 6..]
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    (&part[..idx], args)
                } else {
                    (part, vec![])
                };

            // Build type bindings from receiver: look up the class's type params, bind to args.
            let type_params = self.get_class_type_params(&current_type_fqn).await;
            let receiver_bindings = build_type_bindings(&type_params, &current_type_args);

            let symbols = self
                .try_type_member(&current_type_fqn, method_name, &imports, None)
                .await;
            let resolved = match symbols.into_iter().next() {
                Some(s) => s,
                None => return Some("java.lang.Object".to_string()),
            };

            let meta = resolved.metadata();

            // Build call-site bindings from explicit type args at the call site.
            // TypeBindingPrecedence: receiver > call-site > functional parameter.
            // Receiver bindings are inserted first; call-site entries only fill unbound params.
            let bindings = {
                let call_site_type_params = meta
                    .and_then(|m| m.method_type_params.as_ref())
                    .cloned()
                    .unwrap_or_default();
                let call_site_bindings =
                    build_type_bindings(&call_site_type_params, &call_site_type_args);
                let mut merged = receiver_bindings.clone();
                for (param, bound) in call_site_bindings {
                    merged.entry(param).or_insert(bound);
                }
                merged
            };

            // Prefer generic_return_type (e.g. "E", "Stream<E>") over erased return_type
            let return_type_raw = meta
                .and_then(|m| m.generic_return_type.as_ref().or(m.return_type.as_ref()))
                .cloned();

            current_type_fqn = if let Some(raw) = return_type_raw {
                // Substitute type variables: "E" + {E→"String"} → "String"
                let mut substituted = substitute_type_vars(&raw, &bindings);

                // InferLambdaReturnType: for the last chain step that carries lambda body
                // info, try to bind any remaining type variables from the lambda body.
                // `bindings` already carries receiver + call-site; apply_lambda_return_binding
                // will not override any variable already present (functional param is lowest
                // priority, enforcing receiver > call-site > functional).
                if is_last_step {
                    if let Some(body_info) = lambda_body_info {
                        if let Some(improved) = Box::pin(self.apply_lambda_return_binding(
                            &substituted,
                            &bindings,
                            meta,
                            body_info,
                            lang,
                            tree,
                            content,
                            imports.clone(),
                            position,
                            package_name.clone(),
                            scope_overrides,
                        ))
                        .await
                        {
                            substituted = improved;
                        }
                    }
                }

                // Split the substituted type into name + new args for next iteration
                let (ret_name, ret_args) = parse_type_ref(&substituted);
                current_type_args = ret_args;

                // Resolve the name to FQN
                let parent_package = resolved.package_name().unwrap_or_default().to_string();
                self.resolve_fqn(&ret_name, imports.clone(), Some(parent_package))
                    .await
                    .unwrap_or(ret_name.to_string())
            } else {
                // Type symbol (class/interface) — use FQN directly, no type args
                current_type_args = vec![];
                resolved.fully_qualified_name().to_string()
            };
        }

        Some(current_type_fqn)
    }

    /// Applies InferLambdaReturnType: given a partially-substituted return type that
    /// may still have unbound type variables, tries to bind the output type variable
    /// of the functional parameter using the lambda body's return type.
    ///
    /// `body_info` has the form `"param_name|body_chain"`.
    /// Returns the improved return type string on success, `None` otherwise.
    #[allow(clippy::too_many_arguments)]
    async fn apply_lambda_return_binding(
        &self,
        substituted_return: &str,
        existing_bindings: &HashMap<String, String>,
        meta: Option<&crate::models::symbol::SymbolMetadata>,
        body_info: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
        scope_overrides: &HashMap<String, String>,
    ) -> Option<String> {
        let (param_name, body_chain) = body_info.split_once('|')?;

        // Find a functional parameter type (one with ≥2 type args after substitution).
        let generic_param_types = meta.and_then(|m| m.generic_param_types.as_ref())?;
        let functional_param = generic_param_types
            .iter()
            .map(|pt| substitute_type_vars(pt, existing_bindings))
            .find(|pt| parse_type_ref(pt).1.len() >= 2)?;

        let (_, func_args) = parse_type_ref(&functional_param);

        // Input type = first type arg; output type variable = last type arg.
        let lambda_input_type = func_args.first()?.clone();
        let lambda_output_var = func_args.last()?.clone();

        // Only proceed when the output variable is still unbound (single word, no '<').
        if lambda_output_var.contains('<') || existing_bindings.contains_key(&lambda_output_var) {
            return None;
        }

        // Resolve the lambda input type to an FQN for the scope override.
        let (input_base, _) = parse_type_ref(&lambda_input_type);
        let input_fqn = self
            .resolve_fqn(&input_base, imports.clone(), package_name.clone())
            .await
            .unwrap_or(input_base);

        // Walk the lambda body chain with the parameter type in scope.
        let mut body_scope = scope_overrides.clone();
        body_scope.insert(param_name.to_string(), input_fqn);

        let body_return = Box::pin(self.walk_member_chain_inner(
            body_chain,
            lang,
            tree,
            content,
            imports,
            position,
            package_name,
            &body_scope,
        ))
        .await?;

        // Bind the output type variable to the body return type and re-substitute.
        let mut new_bindings = existing_bindings.clone();
        new_bindings.insert(lambda_output_var, body_return);
        Some(substitute_type_vars(substituted_return, &new_bindings))
    }

    /// Resolves a `__cp__:receiver_chain:method_name:method_param_idx:lambda_param_idx`
    /// marker to the concrete type of the lambda parameter.
    ///
    /// Strategy:
    ///   1. Walk the receiver chain to get the receiver's FQN and generic args.
    ///   2. Look up the method on that type.
    ///   3. From `generic_param_types[method_param_idx]` get the functional param
    ///      type (e.g. `"Function1<T, Unit>"` or `"Consumer<T>"`).
    ///   4. The `lambda_param_idx`-th generic arg of that type is the raw input type.
    ///   5. Substitute receiver generic bindings to get the concrete type.
    #[allow(clippy::too_many_arguments)]
    async fn resolve_closure_param_type(
        &self,
        marker: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
    ) -> Option<String> {
        // marker format: "__cp__:receiver_chain:method_name:method_param_idx:lambda_param_idx"
        let rest = marker.strip_prefix("__cp__:")?;
        // Split into at most 4 parts from the right (method_param_idx and lambda_param_idx
        // are always single tokens; receiver_chain may contain '#' but not ':').
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        if parts.len() != 4 {
            return None;
        }
        let receiver_chain = parts[0];
        let method_name = parts[1];
        let method_param_idx: usize = parts[2].parse().ok()?;
        let lambda_param_idx: usize = parts[3].parse().ok()?;

        // Resolve the receiver to its FQN + generic args.
        let receiver_fqn_str = Box::pin(self.walk_member_chain(
            receiver_chain,
            lang,
            tree,
            content,
            imports.clone(),
            position,
            package_name.clone(),
        ))
        .await?;

        let (receiver_base, receiver_type_args) = parse_type_ref(&receiver_fqn_str);

        // Look up the method on the receiver type.
        let method_symbols = self
            .try_type_member(&receiver_base, method_name, &imports, None)
            .await;
        let method_sym = method_symbols.into_iter().next()?;

        // Get the generic_param_types from the method's metadata.
        let generic_param_types = method_sym
            .metadata()
            .and_then(|m| m.generic_param_types.as_ref())?;

        let functional_param_type = generic_param_types.get(method_param_idx)?;

        // Extract the lambda_param_idx-th type argument as the raw input type.
        let (_, type_args) = parse_type_ref(functional_param_type);
        let raw_input = type_args.get(lambda_param_idx)?.clone();

        // Bind receiver generic params and substitute.
        let receiver_type_params = self.get_class_type_params(&receiver_base).await;
        let bindings = build_type_bindings(&receiver_type_params, &receiver_type_args);
        let concrete = substitute_type_vars(&raw_input, &bindings);

        // Resolve the concrete type name to its FQN.
        let (concrete_base, _) = parse_type_ref(&concrete);
        let method_package = method_sym.package_name().unwrap_or_default().to_string();
        Some(
            self.resolve_fqn(&concrete_base, imports, Some(method_package))
                .await
                .unwrap_or(concrete_base),
        )
    }

    /// Returns the ordered type parameter names for `type_fqn` from the index.
    /// E.g. "java.util.List" → ["E"], "java.util.Map" → ["K", "V"].
    async fn get_class_type_params(&self, type_fqn: &str) -> Vec<String> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };
        if let Ok(Some(sym)) = repo.find_symbol_by_fqn(type_fqn).await {
            if let Some(params) = sym.metadata.0.type_params {
                return params;
            }
        }
        if let Ok(Some(sym)) = repo.find_external_symbol_by_fqn(type_fqn).await {
            if let Some(params) = sym.metadata.0.type_params {
                return params;
            }
        }
        vec![]
    }

    #[allow(clippy::too_many_arguments)]
    /// Returns the JAR paths that are on the classpath of the sub-project owning `file`.
    /// Returns an empty vec for single-project workspaces or when the file cannot be matched.
    async fn jar_paths_for_file(&self, file: &Path) -> Vec<String> {
        let classpath = self.subproject_classpath.read().await;
        classpath
            .iter()
            .find(|entry| entry.contains_file(file))
            .map(|entry| {
                entry
                    .jar_paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn complete_type_member_chain(
        &self,
        qualifier: &str,
        lang: &Arc<dyn LanguageSupport + Send + Sync>,
        tree: &Tree,
        content: &str,
        imports: Vec<String>,
        position: &Position,
        package_name: Option<String>,
        jar_paths: &[String],
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
            tracing::debug!("[LSPINTAR_COMPLETION] walk_member_chain returned None for qualifier={qualifier:?}");
            return vec![];
        };

        tracing::debug!("[LSPINTAR_COMPLETION] resolved fqn={fqn:?} for qualifier={qualifier:?}");

        if let Some(repo) = self.repo.get() {
            if let Ok(symbols) = repo.find_symbols_by_parent_name(&fqn).await
                && !symbols.is_empty()
            {
                tracing::debug!("[LSPINTAR_COMPLETION] found {} project symbols for fqn={fqn:?}", symbols.len());
                return symbols.into_iter().map(ResolvedSymbol::Project).collect();
            }

            let ext_symbols = repo
                .find_external_symbols_by_parent_name_and_jars(&fqn, jar_paths)
                .await
                .unwrap_or_default();
            tracing::debug!("[LSPINTAR_COMPLETION] found {} external symbols for fqn={fqn:?}", ext_symbols.len());
            ext_symbols
                .into_iter()
                .map(ResolvedSymbol::External)
                .collect()
        } else {
            vec![]
        }
    }

    async fn complete_by_prefix(&self, prefix: &str, jar_paths: &[String]) -> Vec<ResolvedSymbol> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };

        let mut symbols: Vec<ResolvedSymbol> = vec![];

        if let Ok(project_syms) = repo.find_symbols_by_prefix(prefix).await {
            symbols.extend(project_syms.into_iter().map(ResolvedSymbol::Project));
        }

        if let Ok(ext_syms) = repo
            .find_external_symbols_by_prefix_and_jars(prefix, jar_paths)
            .await
        {
            symbols.extend(ext_syms.into_iter().map(ResolvedSymbol::External));
        }

        symbols
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

    pub(crate) async fn resolve_symbol_at_position(
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

        let mut imports = lang.get_imports(&tree, &content);
        for imp in lang.get_implicit_imports() {
            if !imports.contains(&imp) {
                imports.push(imp);
            }
        }
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

        let build_tool_guard = self.build_tool.read().await;
        if let Some(bt) = build_tool_guard.as_ref().cloned() {
            drop(build_tool_guard);
            self.write_classpath_manifest(root, &bt).await;
        }
    }

    async fn write_classpath_manifest(
        &self,
        root: &Path,
        build_tool: &Arc<dyn BuildToolHandler + Send + Sync>,
    ) {
        let root_clone = root.to_path_buf();
        let build_tool_clone = Arc::clone(build_tool);
        let entries = tokio::task::spawn_blocking(move || {
            build_tool_clone.get_subproject_classpath(&root_clone)
        })
        .await;

        let entries = match entries {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                lsp_error!("Failed to get subproject classpath: {e}");
                vec![]
            }
            Err(e) => {
                lsp_error!("Task error getting subproject classpath: {e}");
                vec![]
            }
        };

        *self.subproject_classpath.write().await = entries.clone();

        let classpath_path = root.join(CLASSPATH_MANIFEST_PATH_FRAGMENT);
        match serde_json::to_string(&entries) {
            Ok(json) => {
                if let Err(e) = tokio::fs::write(&classpath_path, json).await {
                    lsp_error!("Failed to write classpath manifest: {e}");
                }
            }
            Err(e) => lsp_error!("Failed to serialize classpath manifest: {e}"),
        }
    }

    #[allow(unused)]
    fn needs_full_reindex(&self, root: &Path) -> bool {
        #[cfg(feature = "integration-test")]
        {
            return true;
        }

        #[cfg(not(feature = "integration-test"))]
        {
            let version_path = root.join(INDEX_PATH_FRAGMENT);
            let db_path = root.join(DB_PATH_FRAGMENT);
            let manifest_path = root.join(MANIFEST_PATH_FRAGMENT);
            let classpath_manifest_path = root.join(CLASSPATH_MANIFEST_PATH_FRAGMENT);

            if !manifest_path.exists() || !db_path.exists() || !classpath_manifest_path.exists() {
                return true;
            }

            match std::fs::read_to_string(&version_path) {
                Ok(v) => v.trim() != APP_VERSION,
                Err(_) => true,
            }
        }
    }

    /// Returns the short names of methods that are abstract (or implicitly abstract because they
    /// belong to an interface) in the type identified by `parent_fqn`.
    async fn abstract_method_names(&self, parent_fqn: &str) -> Vec<String> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };

        // Determine whether the parent is an interface (project or external).
        let is_interface = repo
            .find_symbol_by_fqn(parent_fqn)
            .await
            .ok()
            .flatten()
            .map(|s| s.symbol_type == "Interface")
            .or_else(|| {
                None // external lookup requires async, handled below
            })
            .unwrap_or_else(|| {
                false // default: not an interface unless confirmed
            });

        let is_interface = if !is_interface {
            repo.find_external_symbol_by_fqn(parent_fqn)
                .await
                .ok()
                .flatten()
                .map(|s| s.symbol_type == "Interface")
                .unwrap_or(false)
        } else {
            true
        };

        fn is_required(symbol_type: &str, modifiers: &[String], is_interface: bool) -> bool {
            if symbol_type != "Function" {
                return false;
            }
            let has_abstract = modifiers.iter().any(|m| m == "abstract");
            let has_default = modifiers.iter().any(|m| m == "default");
            let has_static = modifiers.iter().any(|m| m == "static");
            has_abstract || (is_interface && !has_default && !has_static)
        }

        let mut names = Vec::new();

        for sym in repo
            .find_symbols_by_parent_name(parent_fqn)
            .await
            .unwrap_or_default()
        {
            if is_required(&sym.symbol_type, &sym.modifiers.0, is_interface) {
                names.push(sym.short_name.clone());
            }
        }

        for sym in repo
            .find_external_symbols_by_parent_name(parent_fqn)
            .await
            .unwrap_or_default()
        {
            if is_required(&sym.symbol_type, &sym.modifiers.0, is_interface) {
                names.push(sym.short_name.clone());
            }
        }

        names
    }

    /// Returns the modifiers of a type (class/interface/enum) identified by its FQN.
    /// Checks project symbols first, then external symbols.
    async fn type_modifiers(&self, fqn: &str) -> Vec<String> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };
        if let Some(sym) = repo.find_symbol_by_fqn(fqn).await.ok().flatten() {
            return sym.modifiers.0.clone();
        }
        if let Some(sym) = repo.find_external_symbol_by_fqn(fqn).await.ok().flatten() {
            return sym.modifiers.0.clone();
        }
        vec![]
    }

    /// Returns the set of all method names reachable on a type (direct + inherited via supers).
    /// Follows the project super-mapping chain one level; also includes direct external methods.
    async fn reachable_method_names(&self, type_fqn: &str) -> HashSet<String> {
        let Some(repo) = self.repo.get() else {
            return HashSet::new();
        };
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue = vec![type_fqn.to_string()];
        let mut names: HashSet<String> = JAVA_OBJECT_METHODS.iter().map(|s| s.to_string()).collect();

        while let Some(fqn) = queue.pop() {
            if !visited.insert(fqn.clone()) {
                continue;
            }
            for sym in repo.find_symbols_by_parent_name(&fqn).await.unwrap_or_default() {
                names.insert(sym.short_name);
            }
            for sym in repo.find_external_symbols_by_parent_name(&fqn).await.unwrap_or_default() {
                names.insert(sym.short_name);
            }
            // Follow project supers one step
            for s in repo.find_supers_by_symbol_fqn(&fqn).await.unwrap_or_default() {
                queue.push(s.fully_qualified_name);
            }
        }
        names
    }

    /// Returns the list of (modifiers, declaring_type_fqn) for all members named `member_name`
    /// that are directly declared on `type_fqn` (no inheritance).
    async fn direct_member_symbols(
        &self,
        type_fqn: &str,
        member_name: &str,
    ) -> Vec<Vec<String>> {
        let Some(repo) = self.repo.get() else {
            return vec![];
        };
        let mut results = Vec::new();
        for sym in repo.find_symbols_by_parent_name(type_fqn).await.unwrap_or_default() {
            if sym.short_name == member_name {
                results.push(sym.modifiers.0.clone());
            }
        }
        for sym in repo.find_external_symbols_by_parent_name(type_fqn).await.unwrap_or_default() {
            if sym.short_name == member_name {
                results.push(sym.modifiers.0.clone());
            }
        }
        results
    }

    /// Walks the supertype chain of `class_fqn` and returns the return type of the first method
    /// named `method_name` found in any direct or inherited supertype.
    async fn parent_method_return_type(
        &self,
        class_fqn: &str,
        method_name: &str,
    ) -> Option<String> {
        let repo = self.repo.get()?;
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue = vec![class_fqn.to_string()];

        while let Some(fqn) = queue.pop() {
            if !visited.insert(fqn.clone()) {
                continue;
            }
            for sym in repo.find_symbols_by_parent_name(&fqn).await.unwrap_or_default() {
                if sym.short_name == method_name && sym.symbol_type == "Function" {
                    return sym
                        .metadata
                        .0
                        .generic_return_type
                        .clone()
                        .or_else(|| sym.metadata.0.return_type.clone());
                }
            }
            for sym in repo
                .find_external_symbols_by_parent_name(&fqn)
                .await
                .unwrap_or_default()
            {
                if sym.short_name == method_name && sym.symbol_type == "Function" {
                    return sym
                        .metadata
                        .0
                        .generic_return_type
                        .clone()
                        .or_else(|| sym.metadata.0.return_type.clone());
                }
            }
            for s in repo.find_supers_by_symbol_fqn(&fqn).await.unwrap_or_default() {
                queue.push(s.fully_qualified_name);
            }
        }
        None
    }

    pub async fn compute_diagnostics(&self, uri: &Url) -> Option<Vec<Diagnostic>> {
        // Suppress diagnostics until the initial index is built; symbol lookups against
        // a half-populated repo produce spurious unresolved/overload errors.
        if !self.index_ready.load(Ordering::Acquire) {
            return Some(vec![]);
        }
        let path = PathBuf::from_str(uri.path()).unwrap();
        let ext = path.extension().and_then(|e| e.to_str())?;
        let lang = self.languages.get(ext)?;
        let parse_result = if let Some(entry) = self.documents.get(&uri.to_string()) {
            lang.parse_str(&entry.0)
        } else {
            lang.parse(&path)
        };
        let (tree, content) = parse_result?;
        Some(self.compute_diagnostics_from_tree(&tree, &content, lang.as_ref()).await)
    }

    async fn compute_diagnostics_from_tree(
        &self,
        tree: &Tree,
        content: &str,
        lang: &dyn lsp_core::language_support::LanguageSupport,
    ) -> Vec<Diagnostic> {

        let mut diagnostics = lang.collect_diagnostics(&tree, &content);

        // Semantic check: unresolved symbols
        let type_refs = lang.get_type_references(&tree, &content);
        if !type_refs.is_empty() {
            if let Some(repo) = self.repo.get() {
                let imports = lang.get_imports(&tree, &content);
                let package = lang.get_package_name(&tree, &content);
                let local_types = lang.get_declared_type_names(&tree, &content);

                for (name, range) in type_refs {
                    if is_type_ref_skippable(&name, &local_types) {
                        continue;
                    }
                    let resolved = self
                        .resolve_fqn_strict(&name, &imports, package.clone())
                        .await;
                    let Some(fqn) = resolved else {
                        continue;
                    };
                    let in_project = repo
                        .find_symbol_by_fqn(&fqn)
                        .await
                        .ok()
                        .flatten()
                        .is_some();
                    let in_external = !in_project
                        && repo
                            .find_external_symbol_by_fqn(&fqn)
                            .await
                            .ok()
                            .flatten()
                            .is_some();
                    if !in_project && !in_external {
                        diagnostics.push(Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "unresolved_symbol".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!("Cannot resolve symbol '{name}'"),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Semantic check: unimplemented abstract methods
        let class_decls = lang.get_class_declarations(&tree, &content);
        if !class_decls.is_empty() {
            let imports = lang.get_imports(&tree, &content);
            let package = lang.get_package_name(&tree, &content);

            for class_data in class_decls {
                if class_data.is_abstract {
                    continue;
                }
                for parent_name in &class_data.parents {
                    let Some(parent_fqn) = self
                        .resolve_fqn(parent_name, imports.clone(), package.clone())
                        .await
                    else {
                        continue;
                    };
                    // final_class_extended: check whether the parent is declared final.
                    let parent_mods = self.type_modifiers(&parent_fqn).await;
                    if parent_mods.iter().any(|m| m == "final") {
                        diagnostics.push(Diagnostic {
                            range: class_data.ident_range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "final_class_extended".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "'{}' cannot extend final class '{}'",
                                class_data.name, parent_name
                            ),
                            ..Default::default()
                        });
                    }

                    let required = self.abstract_method_names(&parent_fqn).await;
                    for method_name in required {
                        if !class_data.defined_method_names.contains(&method_name) {
                            diagnostics.push(Diagnostic {
                                range: class_data.ident_range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: Some(NumberOrString::String(
                                    "unimplemented_abstract_methods".to_string(),
                                )),
                                source: Some("lspintar".to_string()),
                                message: format!(
                                    "'{}' must implement '{}'",
                                    class_data.name, method_name
                                ),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }

        // Semantic check: abstract_class_instantiated
        let object_creations = lang.get_object_creations(&tree, &content);
        if !object_creations.is_empty() {
            if self.repo.get().is_some() {
                let imports = lang.get_imports(&tree, &content);
                let package = lang.get_package_name(&tree, &content);

                for creation in object_creations {
                    let Some(fqn) = self
                        .resolve_fqn(&creation.type_name, imports.clone(), package.clone())
                        .await
                    else {
                        continue;
                    };
                    let mods = self.type_modifiers(&fqn).await;
                    if mods.iter().any(|m| m == "abstract") {
                        diagnostics.push(Diagnostic {
                            range: creation.range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "abstract_class_instantiated".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "Cannot instantiate abstract class '{}'",
                                creation.type_name
                            ),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Semantic checks: method_not_found, inaccessible_member, static_member_via_instance
        let member_accesses = lang.get_member_accesses(&tree, &content);
        if !member_accesses.is_empty() {
            if let Some(_repo) = self.repo.get() {
                let imports = lang.get_imports(&tree, &content);
                let package = lang.get_package_name(&tree, &content);

                for access in member_accesses {
                    let receiver_pos = access.receiver_range.start;
                    let Some(raw_type) =
                        lang.find_variable_type(&tree, &content, &access.receiver_name, &receiver_pos)
                    else {
                        continue;
                    };
                    // Strip generic arguments from the type name (e.g. "List<String>" → "List")
                    let base_type = raw_type
                        .split('<')
                        .next()
                        .unwrap_or(&raw_type)
                        .trim()
                        .to_string();
                    if is_type_ref_skippable(&base_type, &[]) {
                        continue;
                    }
                    let Some(type_fqn) = self
                        .resolve_fqn(&base_type, imports.clone(), package.clone())
                        .await
                    else {
                        continue;
                    };

                    let reachable = self.reachable_method_names(&type_fqn).await;

                    if !reachable.contains(&access.member_name) {
                        // Only emit method_not_found for Java (extension methods in Groovy/Kotlin
                        // cause excessive false positives).
                        if lang.get_language() == Language::Java {
                            diagnostics.push(Diagnostic {
                                range: access.member_range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: Some(NumberOrString::String(
                                    "method_not_found".to_string(),
                                )),
                                source: Some("lspintar".to_string()),
                                message: format!(
                                    "Method '{}' not found on type '{}'",
                                    access.member_name, base_type
                                ),
                                ..Default::default()
                            });
                        }
                        // Skip modifier checks when method isn't found
                        continue;
                    }

                    // Check direct-member modifiers (first overload that exists)
                    let direct = self
                        .direct_member_symbols(&type_fqn, &access.member_name)
                        .await;
                    if direct.is_empty() {
                        // Method exists only on a super — skip modifier checks
                        continue;
                    }

                    // inaccessible_member: all overloads are private
                    let all_private =
                        direct.iter().all(|mods| mods.iter().any(|m| m == "private"));
                    if all_private {
                        diagnostics.push(Diagnostic {
                            range: access.member_range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "inaccessible_member".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "'{}' has private access in '{}'",
                                access.member_name, base_type
                            ),
                            ..Default::default()
                        });
                        continue;
                    }

                    // static_member_via_instance: at least one overload is static but
                    // we're calling it on an instance variable
                    let any_static =
                        direct.iter().any(|mods| mods.iter().any(|m| m == "static"));
                    if any_static {
                        diagnostics.push(Diagnostic {
                            range: access.member_range,
                            severity: Some(DiagnosticSeverity::WARNING),
                            code: Some(NumberOrString::String(
                                "static_member_via_instance".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "Static member '{}' accessed via instance reference",
                                access.member_name
                            ),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Semantic check: wrong_type_argument_count
        let generic_usages = lang.get_generic_type_usages(&tree, &content);
        if !generic_usages.is_empty() {
            if let Some(repo) = self.repo.get() {
                let imports = lang.get_imports(&tree, &content);
                let package = lang.get_package_name(&tree, &content);
                let local_types = lang.get_declared_type_names(&tree, &content);

                for usage in generic_usages {
                    if is_type_ref_skippable(&usage.type_name, &local_types) {
                        continue;
                    }
                    let Some(fqn) = self
                        .resolve_fqn(&usage.type_name, imports.clone(), package.clone())
                        .await
                    else {
                        continue;
                    };
                    // Look up the expected number of type parameters
                    let expected = repo
                        .find_symbol_by_fqn(&fqn)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|s| s.metadata.0.type_params)
                        .or_else(|| {
                            // will be resolved below for external symbols
                            None
                        });
                    let expected = if expected.is_none() {
                        repo.find_external_symbol_by_fqn(&fqn)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|s| s.metadata.0.type_params)
                    } else {
                        expected
                    };
                    let Some(type_params) = expected else {
                        continue; // not a generic type or not indexed
                    };
                    let expected_count = type_params.len();
                    if usage.arg_count != expected_count {
                        diagnostics.push(Diagnostic {
                            range: usage.range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "wrong_type_argument_count".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "'{}' expects {} type argument{}, but {} {} supplied",
                                usage.type_name,
                                expected_count,
                                if expected_count == 1 { "" } else { "s" },
                                usage.arg_count,
                                if usage.arg_count == 1 { "was" } else { "were" },
                            ),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Semantic check: override_incompatible_signature
        let override_methods = lang.get_override_methods(&tree, &content);
        if !override_methods.is_empty() {
            let imports = lang.get_imports(&tree, &content);
            let package = lang.get_package_name(&tree, &content);

            for method in override_methods {
                let Some(class_fqn) = self
                    .resolve_fqn(&method.containing_class, imports.clone(), package.clone())
                    .await
                else {
                    continue;
                };
                let Some(parent_ret_raw) = self
                    .parent_method_return_type(&class_fqn, &method.method_name)
                    .await
                else {
                    continue;
                };
                let parent_base = strip_type_args(&parent_ret_raw);
                if is_unconstrained_return_type(parent_base) {
                    continue;
                }

                let override_base = method
                    .return_type
                    .as_deref()
                    .map(strip_type_args)
                    .unwrap_or("void");
                if is_unconstrained_return_type(override_base) {
                    continue;
                }

                if override_base != parent_base {
                    diagnostics.push(Diagnostic {
                        range: method.range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: Some(NumberOrString::String(
                            "override_incompatible_signature".to_string(),
                        )),
                        source: Some("lspintar".to_string()),
                        message: format!(
                            "'{}' cannot override: return type '{}' is incompatible with '{}'",
                            method.method_name, override_base, parent_base
                        ),
                        ..Default::default()
                    });
                }
            }
        }

        // Semantic check: narrowing_conversion (Java/Groovy — Kotlin skip is justified)
        let narrowing_candidates = lang.get_narrowing_candidates(&tree, &content);
        for candidate in narrowing_candidates {
            let lookup_pos = candidate.range.start;
            let Some(rhs_type_raw) =
                lang.find_variable_type(&tree, &content, &candidate.rhs_name, &lookup_pos)
            else {
                continue;
            };
            let rhs_base = rhs_type_raw.split('<').next().unwrap_or(&rhs_type_raw).trim();
            if is_narrowing_conversion(&candidate.declared_type, rhs_base) {
                diagnostics.push(Diagnostic {
                    range: candidate.range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("narrowing_conversion".to_string())),
                    source: Some("lspintar".to_string()),
                    message: format!(
                        "Narrowing conversion from '{}' to '{}'",
                        rhs_base, candidate.declared_type
                    ),
                    ..Default::default()
                });
            }
        }

        // Semantic check: wrong_argument_types (Java/Groovy/Kotlin)
        let call_sites = lang.get_method_call_sites(&tree, &content);
        if !call_sites.is_empty() {
            if let Some(repo) = self.repo.get() {
                let imports = lang.get_imports(&tree, &content);
                let package = lang.get_package_name(&tree, &content);

                for site in call_sites {
                    let recv_pos = site.receiver_range.start;
                    let Some(raw_recv_type) =
                        lang.find_variable_type(&tree, &content, &site.receiver_name, &recv_pos)
                    else {
                        continue;
                    };
                    let base_recv = raw_recv_type
                        .split('<')
                        .next()
                        .unwrap_or(&raw_recv_type)
                        .trim()
                        .to_string();
                    if is_type_ref_skippable(&base_recv, &[]) {
                        continue;
                    }
                    let Some(recv_fqn) = self
                        .resolve_fqn(&base_recv, imports.clone(), package.clone())
                        .await
                    else {
                        continue;
                    };

                    // Collect all overloads of this method on the receiver type (project + external)
                    let mut overloads: Vec<Vec<String>> = Vec::new();
                    if let Ok(syms) = repo.find_symbols_by_parent_name(&recv_fqn).await {
                        for s in syms {
                            if s.short_name == site.method_name {
                                let param_types: Vec<String> = s
                                    .metadata
                                    .0
                                    .parameters
                                    .unwrap_or_default()
                                    .into_iter()
                                    .filter_map(|p| p.type_name)
                                    .collect();
                                overloads.push(param_types);
                            }
                        }
                    }
                    if let Ok(ext_syms) = repo
                        .find_external_symbols_by_parent_name(&recv_fqn)
                        .await
                    {
                        for s in ext_syms {
                            if s.short_name == site.method_name {
                                let param_types: Vec<String> = s
                                    .metadata
                                    .0
                                    .parameters
                                    .unwrap_or_default()
                                    .into_iter()
                                    .filter_map(|p| p.type_name)
                                    .collect();
                                overloads.push(param_types);
                            }
                        }
                    }

                    if overloads.is_empty() {
                        continue; // method unknown — skip
                    }

                    let arg_count = site.args.len();

                    // Determine types for all arguments that are literals or identifiers.
                    // If any argument type cannot be determined, treat it as compatible (no false positive).
                    let mut arg_bases: Vec<Option<String>> = Vec::new();
                    for arg in &site.args {
                        let base = arg_literal_base_type(&arg.node_kind, &arg.text)
                            .map(|s| s.to_string())
                            .or_else(|| {
                                if arg.node_kind == "identifier" {
                                    lang.find_variable_type(
                                        &tree,
                                        &content,
                                        &arg.text,
                                        &arg.range.start,
                                    )
                                    .map(|t| {
                                        t.split('<')
                                            .next()
                                            .unwrap_or(&t)
                                            .trim()
                                            .to_string()
                                    })
                                } else {
                                    None // complex expression — skip type check
                                }
                            });
                        arg_bases.push(base);
                    }

                    // Check whether any overload is compatible.
                    // An overload is compatible when:
                    //   (a) its arity matches arg_count (varargs ignored — conservative), AND
                    //   (b) every determinable argument type is compatible with the overload's
                    //       corresponding parameter type.
                    let any_overload_matches = overloads.iter().any(|params| {
                        if params.len() != arg_count {
                            return false;
                        }
                        params.iter().enumerate().all(|(i, param_type)| {
                            match arg_bases.get(i) {
                                Some(Some(arg_base)) => {
                                    is_arg_compatible_with_param(arg_base, param_type)
                                }
                                // Unknown arg type → assume compatible
                                _ => true,
                            }
                        })
                    });

                    // If any argument type couldn't be determined, skip — avoids false
                    // positives when the overload index is incomplete or the arg is a
                    // complex expression.
                    let all_args_known = arg_bases.iter().all(|b| b.is_some());

                    if !any_overload_matches && all_args_known {
                        let arg_desc: Vec<String> = arg_bases
                            .iter()
                            .map(|b| b.clone().unwrap_or_else(|| "?".to_string()))
                            .collect();
                        diagnostics.push(Diagnostic {
                            range: site.method_range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String(
                                "wrong_argument_types".to_string(),
                            )),
                            source: Some("lspintar".to_string()),
                            message: format!(
                                "No overload of '{}' matches argument types ({})",
                                site.method_name,
                                arg_desc.join(", "),
                            ),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        diagnostics
    }

    async fn publish_diagnostics(&self, uri: Url) {
        if let Some(diagnostics) = self.compute_diagnostics(&uri).await {
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
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

        self.client
            .register_capability(vec![Registration {
                id: "workspace/didChangeWatchedFiles".to_string(),
                method: "workspace/didChangeWatchedFiles".to_string(),
                register_options: Some(
                    serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                        watchers: vec![
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.groovy".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.java".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.kt".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.kts".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.gradle".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                            FileSystemWatcher {
                                glob_pattern: GlobPattern::String("**/*.gradle.kts".to_string()),
                                kind: Some(WatchKind::all()),
                            },
                        ],
                    })
                    .unwrap(),
                ),
            }])
            .await
            .ok();

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
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
                version: Some(APP_VERSION.to_string()),
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
                let indexing_start = Instant::now();

                let token_ws = format!("idx-ws-{}", uuid::Uuid::new_v4());
                let token_ws_end = token_ws.clone();

                let token_ws_save = format!("idx-ws-save-{}", uuid::Uuid::new_v4());
                let token_ws_save_end = token_ws_save.clone();

                // Show progress before any slow work so the user immediately sees the server is active.
                lsp_progress_begin!(&token_ws, "Preparing index...");

                debug!("Full reindex required, clearing existing index.");
                let _ = tokio::fs::remove_file(root.join(MANIFEST_PATH_FRAGMENT)).await;
                if let Err(e) = repo.clear_all().await {
                    lsp_error!("Failed to clear index: {e}");
                    lsp_progress_end!(&token_ws_end);
                    return;
                }

                lsp_progress!(&token_ws, "Resolving dependencies...", 0.0);
                lsp_info!("Resolving dependencies...");

                let external_deps = match build_tool.get_dependency_paths(&root) {
                    Ok(deps) => deps,
                    Err(e) => {
                        let message = format!("Failed to get dependencies: {e}");
                        lsp_error!("{}", message);
                        panic!("{}", message);
                    }
                };
                let jdk_sources = match build_tool.get_jdk_dependency_path(&root) {
                    Ok(deps) => deps,
                    Err(e) => {
                        let message = format!("Failed to get JDK sources: {e}");
                        lsp_error!("{}", message);
                        panic!("{}", message);
                    }
                };
                let mut jars: Vec<(Option<PathBuf>, Option<PathBuf>)> = external_deps;

                // exclude JDK
                let jars_for_manifest = jars.clone();

                if let Some(src_zip) = jdk_sources {
                    jars.push((None, Some(src_zip)));
                }

                lsp_progress!(&token_ws, "Indexing workspace...", 0.0);

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

                self.write_classpath_manifest(&root, &build_tool).await;

                lsp_info!(
                    "Indexing finished in {:.2}s",
                    indexing_start.elapsed().as_secs_f64()
                );

                // Record the current VCS revision so the next IncrementalOpen knows
                // which files changed since this full reindex.
                if let Ok(rev) = vcs.get_current_revision() {
                    if let Err(e) =
                        tokio::fs::write(root.join(VCS_REVISION_PATH_FRAGMENT), &rev).await
                    {
                        lsp_error!("Failed to write {VCS_REVISION_PATH_FRAGMENT}: {e}");
                    }
                }
            } else {
                // IncrementalOpen: load the persisted classpath manifest into memory.
                let classpath_path = root.join(CLASSPATH_MANIFEST_PATH_FRAGMENT);
                if let Ok(bytes) = tokio::fs::read(&classpath_path).await {
                    if let Ok(entries) = serde_json::from_slice(&bytes) {
                        *self.subproject_classpath.write().await = entries;
                    }
                }

                // Re-index only source files that changed since the last stored VCS revision.
                let stored_rev = tokio::fs::read_to_string(root.join(VCS_REVISION_PATH_FRAGMENT))
                    .await
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                if let Some(stored) = stored_rev {
                    if let Ok(current) = vcs.get_current_revision() {
                        if stored != current {
                            match vcs.get_changed_files(&stored, &current, &root) {
                                Ok(changed) => {
                                    let supported_exts: std::collections::HashSet<&str> =
                                        languages.iter().map(|(k, _)| k.as_str()).collect();
                                    let source_changes: Vec<PathBuf> = changed
                                        .into_iter()
                                        .filter(|p| {
                                            p.extension()
                                                .and_then(|e| e.to_str())
                                                .map(|e| supported_exts.contains(e))
                                                .unwrap_or(false)
                                        })
                                        .collect();

                                    if !source_changes.is_empty() {
                                        lsp_info!(
                                            "IncrementalOpen: re-indexing {} changed file(s) since {}",
                                            source_changes.len(),
                                            &stored[..stored.len().min(8)]
                                        );
                                        for path in source_changes {
                                            let _ = self.debounce_tx.send(path).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    lsp_error!("Failed to get changed files for incremental open: {e}");
                                }
                            }

                            if let Err(e) = tokio::fs::write(
                                root.join(VCS_REVISION_PATH_FRAGMENT),
                                &current,
                            )
                            .await
                            {
                                lsp_error!("Failed to update {VCS_REVISION_PATH_FRAGMENT}: {e}");
                            }
                        }
                    }
                }
            }

            *indexer_lock.write().await = Some(indexer);
            *vcs_handler_lock.write().await = Some(vcs);
            *workspace_root_lock.write().await = Some(root.clone());

            if let Some(vcs) = self.vcs_handler.read().await.as_ref() {
                if let Ok(rev) = vcs.get_current_revision() {
                    *self.last_known_revision.write().await = Some(rev);
                }
            }

            if let Err(e) = tokio::fs::write(root.join(INDEX_PATH_FRAGMENT), APP_VERSION).await {
                lsp_error!("Failed to write {INDEX_PATH_FRAGMENT}: {e}");
            }

            self.index_ready.store(true, Ordering::Release);

            // Publish diagnostics for any files already opened during indexing.
            let open_uris: Vec<Url> = self
                .documents
                .iter()
                .filter_map(|entry| Url::parse(entry.key()).ok())
                .collect();
            for uri in open_uris {
                self.publish_diagnostics(uri).await;
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

        let indexer_guard = self.indexer.read().await;
        let indexer = indexer_guard.as_ref();

        let locations: Vec<Location> = stream::iter(symbols)
            .then(|s| async move {
                let indexer = indexer.clone();
                match s {
                    ResolvedSymbol::External(sym) => {
                        let enriched = sym.with_sources(indexer).await;
                        enriched.as_lsp_location()
                    }
                    other => other.as_lsp_location(),
                }
            })
            .filter_map(|l| async move { l })
            .collect()
            .await;

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

            let mut imports = lang.get_imports(&tree, &content);
            for imp in lang.get_implicit_imports() {
                if !imports.contains(&imp) {
                    imports.push(imp);
                }
            }
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
        let indexer_guard = self.indexer.read().await;
        let indexer = indexer_guard.as_ref().cloned();
        let symbol = match symbols.into_iter().next() {
            Some(ResolvedSymbol::External(sym)) => {
                ResolvedSymbol::External(sym.with_sources(indexer.as_ref()).await)
            }
            Some(other) => other,
            None => return Ok(None),
        };
        Ok(symbol.as_lsp_hover())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        self.documents
            .insert(uri.to_string(), (text, Instant::now()));
        self.publish_diagnostics(uri).await;
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

        self.publish_diagnostics(params.text_document.uri).await;
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
        let cached_content = self
            .documents
            .get(&pos.text_document.uri.to_string())
            .map(|e| e.0.clone());
        let (tree, content) = if let Some(ref text) = cached_content {
            lang.parse_str(text)
        } else {
            lang.parse(&path)
        }
        .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("Failed to parse file"))?;
        let mut imports = lang.get_imports(&tree, &content);
        for imp in lang.get_implicit_imports() {
            if !imports.contains(&imp) {
                imports.push(imp);
            }
        }
        let package_name = lang.get_package_name(&tree, &content);

        let jar_paths = self.jar_paths_for_file(&path).await;

        let line_prefix = if line.is_empty() || char_pos == 0 {
            ""
        } else {
            line.char_indices()
                .nth(char_pos)
                .map(|(i, _)| &line[..i])
                .unwrap_or(&line)
        };
        let mut symbols = if line_prefix.contains('.') {
            let receiver = extract_receiver(&line, char_pos).unwrap_or("");
            self.complete_type_member_chain(
                receiver,
                lang,
                &tree,
                &content,
                imports.clone(),
                &pos.position,
                package_name.clone(),
                &jar_paths,
            )
            .await
        } else {
            let prefix = extract_prefix(&line, char_pos);

            let scope_decls = lang.find_declarations_in_scope(&tree, &content, &pos.position);
            let mut symbols: Vec<ResolvedSymbol> = scope_decls
                .into_iter()
                .filter(|(name, _)| name.starts_with(prefix))
                .map(|(name, var_type)| ResolvedSymbol::Local {
                    uri: params.text_document_position.text_document.uri.clone(),
                    position: pos.position,
                    name,
                    var_type,
                })
                .collect();

            symbols.extend(self.complete_by_prefix(prefix, &jar_paths).await);
            symbols
        };

        symbols.sort_by_key(|s| completion_rank(s, package_name.as_deref()));

        // Deduplicate: keep the first occurrence of each fqn.
        // Multiple JARs can contain the same class; after sorting, the preferred
        // variant (lower rank) is already first.
        let mut seen_fqns = std::collections::HashSet::new();
        symbols.retain(|s| seen_fqns.insert(s.fully_qualified_name().to_string()));

        let items: Vec<CompletionItem> =
            symbols
                .into_iter()
                .filter(|s| s.name() != "<init>")
                .map(|s| match s {
                    ResolvedSymbol::External(_) | ResolvedSymbol::Project(_) => {
                        let is_function = s.node_kind() == lsp_core::node_kind::NodeKind::Function;
                        CompletionItem {
                        label: s.name().to_string(),
                        kind: s.node_kind().to_lsp_kind(),
                        insert_text: if is_function {
                            Some(format!("{}($0)", s.name()))
                        } else {
                            None
                        },
                        insert_text_format: if is_function {
                            Some(InsertTextFormat::SNIPPET)
                        } else {
                            None
                        },
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
                    }
                    }
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

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.rename_impl(params).await
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let text_doc_pos = params.text_document_position;
        let path = PathBuf::from_str(text_doc_pos.text_document.uri.path()).unwrap();
        let position = text_doc_pos.position;

        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_string(),
            None => return Ok(None),
        };
        let Some(lang) = self.languages.get(&ext) else {
            return Ok(None);
        };
        let Some((tree, content)) = lang.parse(&path) else {
            return Ok(None);
        };

        // Identify the symbol name at the cursor.
        let Some((ident, _)) = lang.find_ident_at_position(&tree, &content, &position) else {
            return Ok(None);
        };

        let Some(repo) = self.repo.get() else {
            return Ok(None);
        };
        let file_paths = repo.find_all_source_file_paths().await.unwrap_or_default();

        let mut locations: Vec<Location> = Vec::new();

        for file_path in file_paths {
            let fp = PathBuf::from(&file_path);
            let file_ext = match fp.extension().and_then(|e| e.to_str()) {
                Some(e) => e.to_string(),
                None => continue,
            };
            let Some(file_lang) = self.languages.get(&file_ext) else {
                continue;
            };
            let file_content = match std::fs::read_to_string(&fp) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let Ok(uri) = Url::from_file_path(&fp) else {
                continue;
            };

            let parsed_tree = file_lang.parse_str(&file_content);

            for (line_idx, line) in file_content.lines().enumerate() {
                let mut search_start = 0;
                while let Some(match_pos) = line[search_start..].find(&ident) {
                    let abs = search_start + match_pos;

                    // Word-boundary check: the character before and after must
                    // not be an identifier character (letter, digit, or '_').
                    let is_ident_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                    let before_ok = abs == 0
                        || !is_ident_char(line.as_bytes()[abs - 1]);
                    let after_idx = abs + ident.len();
                    let after_ok = after_idx >= line.len()
                        || !is_ident_char(line.as_bytes()[after_idx]);

                    if before_ok && after_ok {
                        // Skip matches inside comments.
                        if let Some((ref tree, _)) = parsed_tree {
                            if position_in_comment(tree, line_idx, abs) {
                                search_start = abs + 1;
                                if search_start >= line.len() { break; }
                                continue;
                            }
                        }
                        let start = Position {
                            line: line_idx as u32,
                            character: abs as u32,
                        };
                        let end = Position {
                            line: line_idx as u32,
                            character: (abs + ident.len()) as u32,
                        };

                        // Honour include_declaration: skip occurrences in the
                        // same file at the same position as the request.
                        let is_request_site = fp == path
                            && line_idx as u32 == position.line
                            && abs as u32 <= position.character
                            && position.character < end.character;

                        if params.context.include_declaration || !is_request_site {
                            locations.push(Location {
                                uri: uri.clone(),
                                range: Range { start, end },
                            });
                        }
                    }

                    search_start = abs + 1;
                    if search_start >= line.len() {
                        break;
                    }
                }
            }
        }

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .insert(uri.to_string(), (change.text, Instant::now()));
        }
        if let Ok(path) = uri.to_file_path() {
            let _ = self.debounce_tx.send(path).await;
        }
        let _ = self.diag_debounce_tx.send(uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.remove(&uri.to_string());
        self.client.publish_diagnostics(uri, vec![], None).await;
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

                // Skip files currently open in the editor — did_save already re-indexes them.
                if !self.documents.contains_key(&change.uri.to_string()) {
                    let _ = self.debounce_tx.send(path).await;
                }
            }
        }
    }
}
