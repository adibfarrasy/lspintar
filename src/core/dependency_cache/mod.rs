pub mod builtin;
pub mod project_deps;
pub mod source_file_info;
pub mod symbol_index;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use dashmap::DashMap;
use project_deps::ProjectMetadata;
use source_file_info::SourceFileInfo;
use symbol_index::{
    collect_source_files, extract_symbol_definitions, find_project_roots,
    parse_source_files_parallel, SymbolDefinition,
};
use tokio::fs;
use tracing::debug;

use crate::{
    core::{state_manager::set_global, utils::is_project_root},
    lsp_error, lsp_info, lsp_warning,
};

use super::{
    build_tools::{detect_build_tool, find_symbol_in_jar_content, ExternalDependency},
    persistence::PersistenceLayer,
    utils::{find_project_root, is_external_dependency},
};

pub struct DependencyCache {
    // Maps (project_root, fully_qualified_name) -> file locations
    pub symbol_index: Arc<DashMap<(PathBuf, String), PathBuf>>,

    // Maps (project_root, class_name) -> Vec<fully_qualified_name> for wildcard import lookup
    pub class_name_index: Arc<DashMap<(PathBuf, String), Vec<String>>>,

    // Maps builtin class name -> (source_file_path, parsed_tree, source_content)
    pub builtin_infos: Arc<DashMap<String, SourceFileInfo>>,

    // Maps (project_root, type_name) -> Vec<PathBuf>
    pub inheritance_index: Arc<DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>>,

    // Maps (project_root, type_name) -> (source_file_path, parsed_tree, source_content)
    pub project_external_infos: Arc<DashMap<(PathBuf, String), SourceFileInfo>>,

    pub project_metadata: Arc<DashMap<PathBuf, ProjectMetadata>>,

    // Persistence layer for lazy loading
    persistence: Arc<tokio::sync::RwLock<Option<PersistenceLayer>>>,
}

impl DependencyCache {
    pub fn new() -> Self {
        Self {
            symbol_index: Arc::new(DashMap::new()),
            class_name_index: Arc::new(DashMap::new()),
            builtin_infos: Arc::new(DashMap::new()),
            inheritance_index: Arc::new(DashMap::new()),
            project_external_infos: Arc::new(DashMap::new()),
            project_metadata: Arc::new(DashMap::new()),
            persistence: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Initialize persistence layer for lazy loading
    pub async fn initialize_persistence(&self, project_root: PathBuf) -> Result<()> {
        let persistence = PersistenceLayer::new(project_root)?;
        let mut guard = self.persistence.write().await;
        *guard = Some(persistence);
        Ok(())
    }

    /// Check if cache exists and is valid, initialize persistence layer for lazy loading
    pub async fn check_and_initialize_cache(&self, project_root: &PathBuf) -> Result<bool> {
        let persistence = match PersistenceLayer::new(project_root.clone()) {
            Ok(p) => p,
            Err(_) => return Ok(false),
        };

        let is_cache_valid = match persistence.is_git_state_stale() {
            Ok(is_stale) => !is_stale,
            Err(_) => false,
        };

        if is_cache_valid {
            // Load project metadata eagerly as it's needed for dependency resolution
            match persistence.load_project_metadata() {
                Ok(project_metadata_map) => {
                    for (project_path, metadata) in project_metadata_map {
                        self.project_metadata.insert(project_path, metadata);
                    }
                    lsp_info!("lspintar ready");
                }
                Err(e) => {
                    lsp_warning!(
                        "Failed to load project metadata during lazy initialization: {}",
                        e
                    );
                }
            }

            // Cache is valid, initialize persistence for lazy loading
            let mut guard = self.persistence.write().await;
            *guard = Some(persistence);
            drop(guard);

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Save cache to persistent storage
    pub async fn save_to_disk(&self, project_root: &PathBuf) -> Result<()> {
        let persistence = PersistenceLayer::new(project_root.clone())
            .context("Failed to initialize persistence layer")?;

        persistence.store_all_caches(
            &self.symbol_index,
            &self.builtin_infos,
            &self.inheritance_index,
            &self.project_external_infos,
            &self.project_metadata,
        )?;

        Ok(())
    }


    #[tracing::instrument(skip_all)]
    pub async fn index_external_dependency(self: Arc<Self>, current_dir: PathBuf) -> Result<()> {
        tracing::debug!("index_external_dependency called for: {:?}", current_dir);
        self.index_project_symbols(&current_dir)
            .await
            .context("Failed to index project symbols")?;

        let resolver = builtin::BuiltinResolver::new();
        resolver
            .index_builtin_dependencies(self.clone())
            .await
            .context("Failed to index external types")?;

        return Ok(());
    }

    #[tracing::instrument(skip_all)]
    pub async fn index_workspace(self: Arc<Self>, workspace_root: PathBuf) -> Result<()> {
        tracing::debug!(
            "Checking workspace root for project markers: {:?}",
            workspace_root
        );

        // The workspace root provided by the LSP client should be the project root
        // If it's not a direct project root, try to find one within it
        let project_root = if is_project_root(&workspace_root) {
            tracing::debug!("Workspace root is a project root");
            workspace_root.clone()
        } else {
            tracing::debug!("Workspace root is not a project root, searching...");
            find_project_root(&workspace_root).context("Cannot find project root")?
        };

        let build_tool =
            detect_build_tool(project_root.as_path()).context("Cannot detect build tool")?;

        lsp_info!("Starting workspace indexing...");

        let start = Instant::now();
        self.index_project_symbols(&project_root)
            .await
            .context("Failed to index project symbols")?;

        let resolver = builtin::BuiltinResolver::new();
        resolver
            .index_builtin_dependencies(self.clone())
            .await
            .context("Failed to index external types")?;

        debug!("Creating ProjectMapper for build tool: {:?}", build_tool);
        let project_mapper = project_deps::ProjectMapper::new(build_tool.clone());
        debug!(
            "Starting project dependencies indexing for: {:?}",
            project_root
        );
        project_mapper
            .index_project_dependencies(project_root.clone(), self.clone())
            .await
            .context("Failed to index project dependencies")?;
        debug!("Project dependencies indexing completed");

        let duration = start.elapsed();
        lsp_info!("Indexing completed in {:.2}s", duration.as_secs_f64());
        set_global("is_indexing_completed", true);

        if let Err(error) = self.save_to_disk(&project_root).await {
            lsp_error!("Failed to save cache to disk: {}", error);
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn index_project_symbols(&self, current_dir: &PathBuf) -> Result<()> {
        let is_external_dependency = is_external_dependency(current_dir);
        let project_roots = if is_external_dependency {
            vec![current_dir.clone()]
        } else {
            find_project_roots(current_dir).context("Failed to get project roots")?
        };

        for project_root in project_roots {
            let source_files = collect_source_files(&project_root, is_external_dependency)
                .await
                .context("Failed to collect source files")?;

            tracing::debug!(
                "Found {} source files in project_root: {:?}",
                source_files.len(),
                project_root
            );

            let parsed_files = parse_source_files_parallel(source_files)
                .await
                .context("Failed to parse files")?;

            let symbol_definitions = extract_symbol_definitions(parsed_files)
                .await
                .context("Failed to extract symbol definitions")?;

            for symbol in symbol_definitions {
                let key = (project_root.clone(), symbol.fully_qualified_name.clone());
                self.symbol_index.insert(key, symbol.source_file.clone());

                // Update class name index for wildcard import support
                if let Some(class_name) = symbol.fully_qualified_name.split('.').last() {
                    let class_key = (project_root.clone(), class_name.to_string());
                    self.class_name_index
                        .entry(class_key)
                        .or_insert_with(Vec::new)
                        .push(symbol.fully_qualified_name.clone());
                }

                self.index_inheritance(&project_root, &symbol);
            }

            // Also index decompiled content for this project
            self.index_decompiled_content(&project_root).await?;
        }

        Ok(())
    }

    /// Index symbols from decompiled content stored in project_external_infos
    #[tracing::instrument(skip_all)]
    async fn index_decompiled_content(&self, project_root: &PathBuf) -> Result<()> {
        use crate::core::dependency_cache::symbol_index::extract_symbols_from_source_file_info;

        // Get all external info entries for this project
        let external_infos: Vec<_> = self
            .project_external_infos
            .iter()
            .filter_map(|entry| {
                let ((entry_project_root, _fqn), source_info) = (entry.key(), entry.value());
                if entry_project_root == project_root {
                    Some(source_info.clone())
                } else {
                    None
                }
            })
            .collect();

        // Process each decompiled source file
        for source_info in external_infos {
            if let Ok(symbols) = extract_symbols_from_source_file_info(&source_info) {
                for symbol in symbols {
                    let key = (project_root.clone(), symbol.fully_qualified_name.clone());
                    self.symbol_index.insert(key, symbol.source_file.clone());

                    // Update class name index for wildcard import support
                    if let Some(class_name) = symbol.fully_qualified_name.split('.').last() {
                        let class_key = (project_root.clone(), class_name.to_string());
                        self.class_name_index
                            .entry(class_key)
                            .or_insert_with(Vec::new)
                            .push(symbol.fully_qualified_name.clone());
                    }

                    self.index_inheritance(project_root, &symbol);
                }
            }
        }

        Ok(())
    }

    /// Find all fully qualified names for a given class name in a project
    /// Used for wildcard import resolution
    pub fn find_symbols_by_class_name(
        &self,
        project_root: &PathBuf,
        class_name: &str,
    ) -> Vec<String> {
        let class_key = (project_root.clone(), class_name.to_string());
        self.class_name_index
            .get(&class_key)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    /// Synchronous lazy lookup for symbol file path, checking in-memory cache first, then database
    #[tracing::instrument(skip_all)]
    pub fn find_symbol_sync(&self, project_root: &PathBuf, fqn: &str) -> Option<PathBuf> {
        let key = (project_root.clone(), fqn.to_string());

        // First check in-memory cache
        if let Some(file_path) = self.symbol_index.get(&key) {
            return Some(file_path.value().clone());
        }

        // If not found in memory, try database lookup
        let persistence_guard = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.persistence.read())
        });

        if let Some(ref persistence) = *persistence_guard {
            match persistence.lookup_symbol(project_root, fqn) {
                Ok(Some(file_path)) => {
                    // Cache the result in memory for future lookups
                    drop(persistence_guard);
                    self.symbol_index.insert(key, file_path.clone());
                    return Some(file_path);
                }
                Ok(None) => {
                    debug!("symbol '{}' not found in database", fqn);
                }
                Err(e) => {
                    debug!("database query error for symbol '{}': {}", fqn, e);
                }
            }
        } else {
            debug!("no persistence layer available");
        }

        None
    }

    /// Lazy lookup for symbol file path, checking in-memory cache first, then database
    #[tracing::instrument(skip_all)]
    pub async fn find_symbol(&self, project_root: &PathBuf, fqn: &str) -> Option<PathBuf> {
        let key = (project_root.clone(), fqn.to_string());

        // First check in-memory cache
        if let Some(file_path) = self.symbol_index.get(&key) {
            return Some(file_path.value().clone());
        }

        // If not found in memory, try database lookup
        let persistence_guard = self.persistence.read().await;
        if let Some(ref persistence) = *persistence_guard {
            match persistence.lookup_symbol(project_root, fqn) {
                Ok(Some(file_path)) => {
                    // Cache the result in memory for future lookups
                    drop(persistence_guard);
                    self.symbol_index.insert(key, file_path.clone());
                    return Some(file_path);
                }
                Ok(None) => {
                    debug!(
                        "({}, {}) not found in database",
                        project_root.to_str().unwrap_or(""),
                        fqn
                    );
                }
                Err(e) => {
                    debug!("{:#?}", e);
                }
            }
        } else {
            debug!("no persistence layer available");
        }

        None
    }

    /// Synchronous lazy lookup for builtin info, checking in-memory cache first, then database
    #[tracing::instrument(skip_all)]
    pub fn find_builtin_info(&self, class_name: &str) -> Option<SourceFileInfo> {
        // First check in-memory cache
        if let Some(info) = self.builtin_infos.get(class_name) {
            return Some(info.value().clone());
        }

        // If not found in memory, try database lookup
        let persistence_guard = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.persistence.read())
        });

        if let Some(ref persistence) = *persistence_guard {
            match persistence.lookup_builtin_info(class_name) {
                Ok(Some(info)) => {
                    drop(persistence_guard);
                    self.builtin_infos
                        .insert(class_name.to_string(), info.clone());
                    return Some(info);
                }
                Ok(None) => {
                    debug!("builtin '{}' not found in database", class_name);
                }
                Err(e) => {
                    debug!("database query error for builtin '{}': {}", class_name, e);
                }
            }
        } else {
            debug!("no persistence layer available");
        }

        None
    }

    /// Lazy lookup for project external info, checking in-memory cache first, then database
    pub async fn find_project_external_info(
        &self,
        project_root: &PathBuf,
        type_name: &str,
    ) -> Option<SourceFileInfo> {
        let key = (project_root.clone(), type_name.to_string());

        // First check in-memory cache
        if let Some(info) = self.project_external_infos.get(&key) {
            return Some(info.value().clone());
        }

        // If not found in memory, try database lookup
        let persistence_guard = self.persistence.read().await;
        if let Some(ref persistence) = *persistence_guard {
            if let Ok(Some(info)) =
                persistence.lookup_project_external_info(project_root, type_name)
            {
                // Cache the result in memory for future lookups
                drop(persistence_guard);
                self.project_external_infos.insert(key, info.clone());
                return Some(info);
            }
        }

        None
    }

    /// Enhanced external symbol lookup with lazy content parsing fallback
    /// This is the main entry point for finding external symbols with smart fallbacks
    pub async fn find_external_symbol_with_lazy_parsing(
        &self,
        project_root: &PathBuf,
        symbol_name: &str,
    ) -> Option<SourceFileInfo> {
        // 1. First try the standard lookup (fast)
        if let Some(source_info) = self.find_project_external_info(project_root, symbol_name).await {
            return Some(source_info);
        }

        // 2. If not found, try lazy content parsing in project's JARs
        if let Some(project_metadata) = self.project_metadata.get(project_root) {
            // Get all external class names for this project to understand which JARs to check
            for external_class_name in project_metadata.external_dep_names.iter() {
                // Try to find a JAR that contains this class - this tells us which JARs this project uses
                if let Some((jar_path, dependency)) = self.find_jar_for_class(project_root, external_class_name.key()).await {
                    // Now search this JAR for our target symbol using lazy content parsing
                    if let Some(internal_path) = find_symbol_in_jar_content(&jar_path, symbol_name) {
                        debug!("Found {} via lazy content parsing in JAR: {:?}", symbol_name, jar_path);
                        
                        // Create a new SourceFileInfo for this discovered symbol
                        let source_info = SourceFileInfo::new_for_decompilation(
                            jar_path,
                            Some(internal_path),
                            dependency,
                        );

                        // Cache it for future lookups
                        let key = (project_root.clone(), symbol_name.to_string());
                        self.project_external_infos.insert(key, source_info.clone());
                        
                        return Some(source_info);
                    }
                }
            }
        }

        None
    }

    /// Helper function to find which JAR contains a specific class
    async fn find_jar_for_class(
        &self, 
        project_root: &PathBuf, 
        class_name: &str
    ) -> Option<(PathBuf, Option<ExternalDependency>)> {
        let key = (project_root.clone(), class_name.to_string());
        if let Some(source_info) = self.project_external_infos.get(&key) {
            return Some((source_info.source_path.clone(), source_info.dependency.clone()));
        }
        None
    }

    /// Lazy lookup for inheritance index, checking in-memory cache first, then database
    #[tracing::instrument(skip_all)]
    pub async fn find_inheritance_implementations(
        &self,
        project_root: &PathBuf,
        type_name: &str,
    ) -> Option<Vec<(PathBuf, usize, usize)>> {
        let key = (project_root.clone(), type_name.to_string());

        // First check in-memory cache
        if let Some(implementations) = self.inheritance_index.get(&key) {
            return Some(implementations.value().clone());
        }

        // If not found in memory, try database lookup
        let persistence_guard = self.persistence.read().await;
        if let Some(ref persistence) = *persistence_guard {
            match persistence.load_inheritance_index() {
                Ok(inheritance_index_map) => {
                    // Cache all loaded inheritance data in memory for future lookups
                    for entry in inheritance_index_map.iter() {
                        let (db_key, db_implementations) = (entry.key(), entry.value());
                        self.inheritance_index
                            .insert(db_key.clone(), db_implementations.clone());
                    }
                    drop(persistence_guard);

                    // Now check if we have the requested type
                    if let Some(implementations) = self.inheritance_index.get(&key) {
                        return Some(implementations.value().clone());
                    }
                }
                Err(e) => {
                    debug!("database query error: {}", e);
                }
            }
        } else {
            debug!("no persistence layer available");
        }

        None
    }

    #[tracing::instrument(skip_all)]
    pub async fn dump_to_file(&self) -> Result<()> {
        let home_dir = dirs::home_dir().context("Failed to get home directory")?;

        let dump_file = home_dir.join("lsp_index.json");

        let serializable_data = self.convert_to_json_format();

        let json_content = serde_json::to_string_pretty(&serializable_data)
            .context("Failed to serialize symbol index")?;

        fs::write(&dump_file, json_content)
            .await
            .context("Failed to write dump file")?;

        Ok(())
    }

    fn convert_to_json_format(&self) -> serde_json::Value {
        let mut projects = HashMap::new();

        // Group symbols by project root for better readability
        for entry in self.symbol_index.iter() {
            let ((project_root, symbol_name), file_path) = (entry.key(), entry.value());

            let project_key = project_root.to_string_lossy().to_string();
            let file_value = file_path.to_string_lossy().to_string();

            projects
                .entry(project_key)
                .or_insert_with(HashMap::new)
                .insert(symbol_name.clone(), file_value);
        }

        // let mut external_dependencies = HashMap::new();
        // for entry in self.builtin_infos.iter() {
        //     let (class_name, external_info) = (entry.key(), entry.value());
        //     external_dependencies.insert(
        //         class_name.clone(),
        //         serde_json::json!({
        //             "source_file": external_info.source_path.to_string_lossy(),
        //         }),
        //     );
        // }

        // let mut project_external_dependencies = HashMap::new();
        // for entry in self.project_external_infos.iter() {
        //     let ((project_root, class_name), external_info) = (entry.key(), entry.value());
        //     let project_key = project_root.to_string_lossy().to_string();
        //
        //     project_external_dependencies
        //         .entry(project_key)
        //         .or_insert_with(HashMap::new)
        //         .insert(
        //             class_name.clone(),
        //             serde_json::json!({
        //                 "source_file": external_info.source_path.to_string_lossy(),
        //                 "zip_internal_path": external_info.zip_internal_path,
        //             }),
        //         );
        // }

        let mut project_metadata = HashMap::new();
        for entry in self.project_metadata.iter() {
            let (project_root, metadata) = (entry.key(), entry.value());
            let project_key = project_root.to_string_lossy().to_string();

            project_metadata.insert(
                project_key,
                serde_json::json!({
                    "inter_project_deps": metadata.inter_project_deps.iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect::<Vec<_>>(),
                    "external_dep_names_count": metadata.external_dep_names.len(),
                    "indexing_status": format!("{:?}", metadata.indexing_status),
                }),
            );
        }

        serde_json::json!({
            "symbol_index": projects,
            // "external_infos": external_dependencies,
            // "project_external_infos": project_external_dependencies,
            "project_metadata": project_metadata,
            "total_symbols": self.symbol_index.len(),
            "total_external": self.builtin_infos.len(),
            "total_project_external": self.project_external_infos.len(),
            "generated_at": chrono::Utc::now().to_rfc3339()
        })
    }

    fn index_inheritance(&self, project_root: &PathBuf, symbol: &SymbolDefinition) {
        if let Some(parent_class) = &symbol.extends {
            self.inheritance_index
                .entry((project_root.clone(), parent_class.clone()))
                .or_insert_with(Vec::new)
                .push((symbol.source_file.clone(), symbol.line, symbol.column));
        }

        for interface in &symbol.implements {
            self.inheritance_index
                .entry((project_root.clone(), interface.clone()))
                .or_insert_with(Vec::new)
                .push((symbol.source_file.clone(), symbol.line, symbol.column));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct DependencyCacheTestCase {
        name: &'static str,
        setup: fn() -> DependencyCache,
        input: DependencyCacheTestInput,
        expected: DependencyCacheTestExpected,
    }

    struct DependencyCacheTestInput {
        project_root: PathBuf,
        class_name: String,
        fqn: String,
    }

    struct DependencyCacheTestExpected {
        should_find_symbol: bool,
        should_find_class_name: bool,
        class_name_count: usize,
    }

    #[tokio::test]
    async fn test_dependency_cache_operations() {
        let test_cases = vec![
            DependencyCacheTestCase {
                name: "empty cache returns no symbols",
                setup: || DependencyCache::new(),
                input: DependencyCacheTestInput {
                    project_root: PathBuf::from("/test/project"),
                    class_name: "TestClass".to_string(),
                    fqn: "com.example.TestClass".to_string(),
                },
                expected: DependencyCacheTestExpected {
                    should_find_symbol: false,
                    should_find_class_name: false,
                    class_name_count: 0,
                },
            },
            DependencyCacheTestCase {
                name: "finds symbol after insertion",
                setup: || {
                    let cache = DependencyCache::new();
                    let project_root = PathBuf::from("/test/project");
                    let fqn = "com.example.TestClass".to_string();
                    let file_path = PathBuf::from("/test/project/TestClass.groovy");

                    cache
                        .symbol_index
                        .insert((project_root.clone(), fqn.clone()), file_path);
                    cache
                        .class_name_index
                        .entry((project_root, "TestClass".to_string()))
                        .or_insert_with(Vec::new)
                        .push(fqn);
                    cache
                },
                input: DependencyCacheTestInput {
                    project_root: PathBuf::from("/test/project"),
                    class_name: "TestClass".to_string(),
                    fqn: "com.example.TestClass".to_string(),
                },
                expected: DependencyCacheTestExpected {
                    should_find_symbol: true,
                    should_find_class_name: true,
                    class_name_count: 1,
                },
            },
            DependencyCacheTestCase {
                name: "handles multiple classes with same name",
                setup: || {
                    let cache = DependencyCache::new();
                    let project_root = PathBuf::from("/test/project");

                    // Insert two classes with same simple name but different packages
                    let fqn1 = "com.example.util.Helper".to_string();
                    let fqn2 = "com.other.Helper".to_string();
                    let file1 = PathBuf::from("/test/project/util/Helper.groovy");
                    let file2 = PathBuf::from("/test/project/other/Helper.groovy");

                    cache
                        .symbol_index
                        .insert((project_root.clone(), fqn1.clone()), file1);
                    cache
                        .symbol_index
                        .insert((project_root.clone(), fqn2.clone()), file2);

                    let helpers = vec![fqn1, fqn2];
                    cache
                        .class_name_index
                        .insert((project_root, "Helper".to_string()), helpers);
                    cache
                },
                input: DependencyCacheTestInput {
                    project_root: PathBuf::from("/test/project"),
                    class_name: "Helper".to_string(),
                    fqn: "com.example.util.Helper".to_string(),
                },
                expected: DependencyCacheTestExpected {
                    should_find_symbol: true,
                    should_find_class_name: true,
                    class_name_count: 2,
                },
            },
        ];

        for test_case in test_cases {
            debug!("Running test: {}", test_case.name);

            let cache = (test_case.setup)();
            let input = &test_case.input;
            let expected = &test_case.expected;

            // Test symbol lookup
            let symbol_key = (input.project_root.clone(), input.fqn.clone());
            let found_symbol = cache.symbol_index.get(&symbol_key).is_some();
            assert_eq!(
                found_symbol, expected.should_find_symbol,
                "Test '{}': symbol lookup failed",
                test_case.name
            );

            // Test class name lookup
            let class_names =
                cache.find_symbols_by_class_name(&input.project_root, &input.class_name);
            let found_class_name = !class_names.is_empty();
            assert_eq!(
                found_class_name, expected.should_find_class_name,
                "Test '{}': class name lookup failed",
                test_case.name
            );
            assert_eq!(
                class_names.len(),
                expected.class_name_count,
                "Test '{}': class name count mismatch",
                test_case.name
            );
        }
    }

    #[test]
    fn test_dependency_cache_creation() {
        let cache = DependencyCache::new();

        assert_eq!(cache.symbol_index.len(), 0);
        assert_eq!(cache.class_name_index.len(), 0);
        assert_eq!(cache.builtin_infos.len(), 0);
        assert_eq!(cache.inheritance_index.len(), 0);
        assert_eq!(cache.project_external_infos.len(), 0);
        assert_eq!(cache.project_metadata.len(), 0);
    }

    struct InheritanceTestCase {
        name: &'static str,
        symbol: SymbolDefinition,
        expected_inheritance_entries: usize,
    }

    #[test]
    fn test_inheritance_indexing() {
        let test_cases = vec![
            InheritanceTestCase {
                name: "class with no inheritance",
                symbol: SymbolDefinition {
                    fully_qualified_name: "com.example.SimpleClass".to_string(),
                    source_file: PathBuf::from("/test/SimpleClass.groovy"),
                    line: 1,
                    column: 0,
                    extends: None,
                    implements: vec![],
                },
                expected_inheritance_entries: 0,
            },
            InheritanceTestCase {
                name: "class with superclass only",
                symbol: SymbolDefinition {
                    fully_qualified_name: "com.example.ChildClass".to_string(),
                    source_file: PathBuf::from("/test/ChildClass.groovy"),
                    line: 1,
                    column: 0,
                    extends: Some("com.example.BaseClass".to_string()),
                    implements: vec![],
                },
                expected_inheritance_entries: 1,
            },
            InheritanceTestCase {
                name: "class with interfaces only",
                symbol: SymbolDefinition {
                    fully_qualified_name: "com.example.ImplClass".to_string(),
                    source_file: PathBuf::from("/test/ImplClass.groovy"),
                    line: 1,
                    column: 0,
                    extends: None,
                    implements: vec![
                        "com.example.Interface1".to_string(),
                        "com.example.Interface2".to_string(),
                    ],
                },
                expected_inheritance_entries: 2,
            },
            InheritanceTestCase {
                name: "class with both superclass and interfaces",
                symbol: SymbolDefinition {
                    fully_qualified_name: "com.example.ComplexClass".to_string(),
                    source_file: PathBuf::from("/test/ComplexClass.groovy"),
                    line: 1,
                    column: 0,
                    extends: Some("com.example.BaseClass".to_string()),
                    implements: vec![
                        "com.example.Interface1".to_string(),
                        "com.example.Interface2".to_string(),
                    ],
                },
                expected_inheritance_entries: 3,
            },
        ];

        for test_case in test_cases {
            debug!("Running inheritance test: {}", test_case.name);

            let cache = DependencyCache::new();
            let project_root = PathBuf::from("/test/project");

            cache.index_inheritance(&project_root, &test_case.symbol);

            assert_eq!(
                cache.inheritance_index.len(),
                test_case.expected_inheritance_entries,
                "Test '{}': inheritance entry count mismatch",
                test_case.name
            );
        }
    }
}
