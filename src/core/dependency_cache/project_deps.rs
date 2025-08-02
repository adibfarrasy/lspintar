use anyhow::Result;
use dashmap::DashSet;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tracing::debug;

use crate::core::build_tools::{
    execute_gradle_dependencies, parse_gradle_dependencies_output, parse_settings_gradle, BuildTool,
};

use super::DependencyCache;

#[derive(Debug, Clone)]
pub struct ProjectMetadata {
    // other project roots a project depends on
    pub inter_project_deps: Arc<DashSet<PathBuf>>,

    // External class names available to a project
    pub external_dep_names: Arc<DashSet<String>>,

    pub build_file_hash: String,
    pub indexing_status: IndexingStatus,
}

#[derive(Debug, Clone)]
pub enum IndexingStatus {
    InProgress,
    Completed,
    Failed(String),
}

pub struct ProjectMapper {
    build_tool: BuildTool,
}

impl ProjectMapper {
    pub fn new(build_tool: BuildTool) -> Self {
        Self { build_tool }
    }

    #[tracing::instrument(skip_all)]
    pub async fn index_project_dependencies(
        &self,
        project_root: PathBuf,
        cache: Arc<DependencyCache>,
    ) -> Result<()> {
        cache.project_metadata.insert(
            project_root.clone(),
            ProjectMetadata {
                inter_project_deps: Arc::new(DashSet::new()),
                external_dep_names: Arc::new(DashSet::new()),
                build_file_hash: String::new(),
                indexing_status: IndexingStatus::InProgress,
            },
        );

        let result = match self.build_tool {
            BuildTool::Gradle => {
                self.index_project_dependencies_gradle(project_root.clone(), cache.clone())
                    .await
            }
        };

        if let Some(mut metadata) = cache.project_metadata.get_mut(&project_root) {
            match result {
                Ok(_) => metadata.indexing_status = IndexingStatus::Completed,
                Err(ref e) => metadata.indexing_status = IndexingStatus::Failed(e.to_string()),
            }
        }

        result
    }

    #[tracing::instrument(skip_all)]
    async fn index_project_dependencies_gradle(
        &self,
        project_root: PathBuf,
        cache: Arc<DependencyCache>,
    ) -> Result<()> {
        let project_map = parse_settings_gradle(&project_root).await?;
        debug!("Found {} projects in settings.gradle", project_map.len());

        let gradle_tasks: Vec<_> = project_map
            .iter()
            .map(|(project_name, subproject_path)| {
                let project_name = project_name.clone();
                let subproject_path = subproject_path.clone();
                tokio::spawn(async move {
                    let result = execute_gradle_dependencies(&subproject_path).await;
                    (project_name, result)
                })
            })
            .collect();

        let gradle_results = futures::future::join_all(gradle_tasks).await;

        let mut all_gradle_results = HashMap::new();
        for task_result in gradle_results {
            let (project_name, gradle_result) =
                task_result.map_err(|e| anyhow::anyhow!("Task join error: {}", e))?;

            match gradle_result {
                Ok(result) => {
                    debug!("Gradle dependencies completed for project {}", project_name);
                    all_gradle_results.insert(project_name, result);
                }
                Err(e) => {
                    debug!(
                        "Failed to get dependencies for project {}: {}",
                        project_name, e
                    );
                    // Continue with other projects instead of failing entire operation
                }
            }
        }

        let mut all_parsed_deps = HashMap::new();
        for (project_name, gradle_result) in &all_gradle_results {
            let parsed_deps = parse_gradle_dependencies_output(gradle_result)?;
            debug!(
                "Project {} - parsed {} external deps, {} project deps",
                project_name,
                parsed_deps.external_dependencies.len(),
                parsed_deps.project_dependencies.len()
            );
            all_parsed_deps.insert(
                project_name.clone(),
                (
                    parsed_deps.external_dependencies,
                    parsed_deps.project_dependencies,
                ),
            );
        }

        // Step 6: Resolve external dependency class names
        // For each external dep, find the JAR in gradle cache
        // Extract class names from JARs (or use existing external indexing)
        // TODO: Implement resolve_external_dependency_classes()

        // Step 7: Update project metadata in cache
        // Populate inter_project_deps with resolved project paths
        // Populate external_dep_names with class names from external deps
        // Update build_file_hash and indexing_status
        // TODO: Implement update_project_metadata_cache()

        Ok(())
    }
}
