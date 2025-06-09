mod builtin;
pub mod symbol_index;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use builtin::BuiltinTypeInfo;
use dashmap::DashMap;
use log::debug;
use symbol_index::{
    collect_source_files, extract_symbol_definitions, find_project_roots,
    parse_source_files_parallel,
};
use tokio::fs;

pub struct DependencyCache {
    // Maps project root -> resolved classpath entries
    pub classpaths: Arc<DashMap<PathBuf, Vec<PathBuf>>>,
    // Maps (project_root, fully_qualified_name) -> file locations
    pub symbol_index: Arc<DashMap<(PathBuf, String), PathBuf>>,

    // Maps builtin class name -> (source_file_path, parsed_tree, source_content)
    pub builtin_trees: Arc<DashMap<String, BuiltinTypeInfo>>,
    // Maps package pattern -> source directory (java.lang.* -> /path/to/java/lang/)
    pub builtin_packages: Arc<DashMap<String, PathBuf>>,
}

impl DependencyCache {
    pub fn new() -> Self {
        Self {
            classpaths: Arc::new(DashMap::new()),
            symbol_index: Arc::new(DashMap::new()),
            builtin_trees: Arc::new(DashMap::new()),
            builtin_packages: Arc::new(DashMap::new()),
        }
    }

    pub async fn index_workspace(&self) -> Result<()> {
        // Index classpath (read build.gradle, pom.xml, etc.)
        self.index_classpaths().await?;

        // Index all source files in the project
        self.index_project_symbols().await?;

        // Index builtin types (java.lang.*, groovy.lang.*)
        self.index_builtin_types().await?;

        Ok(())
    }

    async fn index_classpaths(&self) -> Result<()> {
        // TODO: implement
        Ok(())
    }

    async fn index_project_symbols(&self) -> Result<()> {
        let project_roots = find_project_roots().await?;

        for project_root in project_roots {
            let source_files = collect_source_files(&project_root)
                .await
                .context("failed to collect source_files")?;

            let parsed_files = parse_source_files_parallel(source_files)
                .await
                .context("failed to parse files")?;

            let symbol_definitions = extract_symbol_definitions(parsed_files)
                .await
                .context("failed to extract symbol definitions")?;

            for symbol in symbol_definitions {
                let key = (project_root.clone(), symbol.fully_qualified_name);
                self.symbol_index.insert(key, symbol.source_file);
            }
        }

        let _ = self.dump_to_file().await;

        Ok(())
    }

    async fn dump_to_file(&self) -> Result<()> {
        let home_dir = dirs::home_dir().context("Failed to get home directory")?;

        let dump_file = home_dir.join("lsp_symbol_index.json");

        let serializable_data = self.convert_symbol_index_to_json_format();

        let json_content = serde_json::to_string_pretty(&serializable_data)
            .context("Failed to serialize symbol index")?;

        fs::write(&dump_file, json_content)
            .await
            .inspect_err(|e| debug!("error writing to file: {e}"))?;

        Ok(())
    }

    fn convert_symbol_index_to_json_format(&self) -> serde_json::Value {
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

        serde_json::json!({
            "symbol_index": projects,
            "total_symbols": self.symbol_index.len(),
            "generated_at": chrono::Utc::now().to_rfc3339()
        })
    }

    async fn index_builtin_types(&self) -> Result<()> {
        // TODO: implement
        Ok(())
    }
}
