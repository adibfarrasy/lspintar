pub mod external;
pub mod project_deps;
pub mod symbol_index;

use std::{collections::HashMap, env, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use external::SourceFileInfo;
use project_deps::ProjectMetadata;
use symbol_index::{
    collect_source_files, extract_symbol_definitions, find_project_roots,
    parse_source_files_parallel, SymbolDefinition,
};
use tokio::fs;
use tracing::debug;

use crate::core::utils::is_project_root;

use super::{build_tools::detect_build_tool, utils::find_project_root};

pub struct DependencyCache {
    // Maps (project_root, fully_qualified_name) -> file locations
    pub symbol_index: Arc<DashMap<(PathBuf, String), PathBuf>>,

    // Maps builtin class name -> (source_file_path, parsed_tree, source_content)
    pub external_infos: Arc<DashMap<String, SourceFileInfo>>,

    // Maps (project_root, type_name) -> Vec<PathBuf>
    pub inheritance_index: Arc<DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>>,

    pub project_external_infos: Arc<DashMap<(PathBuf, String), SourceFileInfo>>,

    pub project_metadata: Arc<DashMap<PathBuf, ProjectMetadata>>,
}

impl DependencyCache {
    pub fn new() -> Self {
        Self {
            symbol_index: Arc::new(DashMap::new()),
            external_infos: Arc::new(DashMap::new()),
            inheritance_index: Arc::new(DashMap::new()),
            project_external_infos: Arc::new(DashMap::new()),
            project_metadata: Arc::new(DashMap::new()),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn index_workspace(self: Arc<Self>) -> Result<()> {
        let current_dir = env::current_dir()?;
        let project_root = if is_project_root(&current_dir) {
            current_dir
        } else {
            find_project_root(&current_dir.to_path_buf()).context("Cannot find project root")?
        };

        let build_tool =
            detect_build_tool(project_root.as_path()).context("Cannot detect build tool")?;

        let start = Instant::now();
        debug!("Starting workspace indexing...");

        let symbols_start = Instant::now();
        self.index_project_symbols()
            .await
            .inspect_err(|e| debug!("Failed to index project symbols: {e}"))?;
        debug!("Symbol indexing took: {:?}", symbols_start.elapsed());

        let ext_dependency_start = Instant::now();

        debug!("build_tool: {:#?}", &build_tool);

        let resolver = external::DependencyResolver::new(&build_tool);
        resolver
            .index_external_dependencies(self.clone())
            .await
            .inspect_err(|e| tracing::debug!("Failed to index external types: {e}"))?;
        debug!(
            "External dependency indexing took: {:?}",
            ext_dependency_start.elapsed()
        );

        let project_deps_start = Instant::now();
        let project_mapper = project_deps::ProjectMapper::new(build_tool.clone());
        project_mapper
            .index_project_dependencies(project_root, self.clone())
            .await
            .inspect_err(|e| debug!("Failed to index project dependencies: {e}"))?;
        debug!(
            "Project dependency indexing took: {:?}",
            project_deps_start.elapsed()
        );

        let total_time = start.elapsed();
        debug!("Total workspace indexing completed in: {:?}", total_time);

        let _ = self.dump_to_file().await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn index_project_symbols(&self) -> Result<()> {
        let project_roots = find_project_roots()
            .await
            .inspect_err(|e| debug!("Failed to get project roots: {e}"))?;

        for project_root in project_roots {
            let source_files = collect_source_files(&project_root)
                .await
                .inspect_err(|e| debug!("Failed to collect source_files: {e}"))?;

            let parsed_files = parse_source_files_parallel(source_files)
                .await
                .inspect_err(|e| debug!("Failed to parse files: {e}"))?;

            let symbol_definitions = extract_symbol_definitions(parsed_files)
                .await
                .inspect_err(|e| debug!("Failed to extract symbol definitions: {e}"))?;

            for symbol in symbol_definitions {
                let key = (project_root.clone(), symbol.fully_qualified_name.clone());
                self.symbol_index.insert(key, symbol.source_file.clone());

                self.index_inheritance(&project_root, &symbol);
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn dump_to_file(&self) -> Result<()> {
        let home_dir = dirs::home_dir().with_context(|| {
            debug!("Failed to get home directory");
            anyhow!("Failed to get home directory")
        })?;

        let dump_file = home_dir.join("lsp_index.json");

        let serializable_data = self.convert_to_json_format();

        let json_content = serde_json::to_string_pretty(&serializable_data).with_context(|| {
            debug!("Failed to serialize to JSON");
            anyhow!("Failed to serialize symbol index")
        })?;

        fs::write(&dump_file, json_content)
            .await
            .inspect_err(|e| debug!("error writing to file: {e}"))?;

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

        let mut external_dependencies = HashMap::new();
        for entry in self.external_infos.iter() {
            let (class_name, external_info) = (entry.key(), entry.value());
            external_dependencies.insert(
                class_name.clone(),
                serde_json::json!({
                    "source_file": external_info.source_path.to_string_lossy(),
                }),
            );
        }

        let mut project_external_dependencies = HashMap::new();
        for entry in self.project_external_infos.iter() {
            let ((project_root, class_name), external_info) = (entry.key(), entry.value());
            let project_key = project_root.to_string_lossy().to_string();

            project_external_dependencies
                .entry(project_key)
                .or_insert_with(HashMap::new)
                .insert(
                    class_name.clone(),
                    serde_json::json!({
                        "source_file": external_info.source_path.to_string_lossy(),
                        "zip_internal_path": external_info.zip_internal_path,
                    }),
                );
        }

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
            "external_infos": external_dependencies,
            "project_external_infos": project_external_dependencies,
            "project_metadata": project_metadata,
            "total_symbols": self.symbol_index.len(),
            "total_external": self.external_infos.len(),
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
