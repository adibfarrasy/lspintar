use anyhow::Result;
use dashmap::DashSet;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tracing::debug;

use crate::core::build_tools::{
    execute_gradle_dependencies, extract_class_names_from_jar, find_jar_in_gradle_cache,
    index_jar_sources, parse_gradle_dependencies_output, parse_settings_gradle, BuildTool,
    ExternalDependency,
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
        let root_gradle_result = execute_gradle_dependencies(&project_root).await?;
        all_gradle_results.insert("".to_string(), root_gradle_result);

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

        for (project_name, (external_deps, project_deps)) in &all_parsed_deps {
            let current_project_path = if project_name.is_empty() || project_name == ":" {
                // Root project
                project_root.clone()
            } else {
                project_map.get(project_name).cloned().unwrap_or_else(|| {
                    debug!("Project path not found for {}, using root", project_name);
                    project_root.clone()
                })
            };

            let class_names = self
                .resolve_and_index_external_dependencies(
                    external_deps,
                    &current_project_path,
                    cache.clone(),
                )
                .await?;

            cache
                .project_metadata
                .entry(current_project_path.clone())
                .or_insert_with(|| ProjectMetadata {
                    inter_project_deps: Arc::new(DashSet::new()),
                    external_dep_names: Arc::new(DashSet::new()),
                    build_file_hash: String::new(),
                    indexing_status: IndexingStatus::InProgress,
                });

            if let Some(mut metadata) = cache.project_metadata.get_mut(&current_project_path) {
                metadata.external_dep_names.clear();
                for class_name in class_names {
                    metadata.external_dep_names.insert(class_name);
                }

                metadata.inter_project_deps.clear();
                for project_ref in project_deps {
                    if let Some(project_path) = project_map.get(project_ref) {
                        metadata.inter_project_deps.insert(project_path.clone());
                    }
                }

                metadata.indexing_status = IndexingStatus::Completed;

                debug!(
                    "Updated metadata for project {}: {} external classes, {} project deps",
                    project_name,
                    metadata.external_dep_names.len(),
                    metadata.inter_project_deps.len()
                );
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_and_index_external_dependencies(
        &self,
        external_deps: &[ExternalDependency],
        project_path: &PathBuf,
        cache: Arc<DependencyCache>,
    ) -> Result<HashSet<String>> {
        let mut all_class_names = HashSet::new();

        let chunk_size = std::cmp::max(1, external_deps.len() / num_cpus::get());
        let mut handles = Vec::new();

        for chunk in external_deps.chunks(chunk_size) {
            let chunk = chunk.to_vec();
            let project_path = project_path.clone();
            let cache = cache.clone();

            let handle = tokio::task::spawn(async move {
                let mut chunk_classes = HashSet::new();

                for dep in chunk {
                    if let Some(jar_path) = find_jar_in_gradle_cache(&dep).await {
                        if let Ok(classes) = extract_class_names_from_jar(&jar_path).await {
                            chunk_classes.extend(classes.clone());

                            if let Err(e) =
                                index_jar_sources(&jar_path, &project_path, &cache, &classes).await
                            {
                                debug!("Failed to index JAR sources for {:?}: {}", jar_path, e);
                            }
                        }
                    }
                }

                chunk_classes
            });
            handles.push(handle);
        }

        for handle in handles {
            if let Ok(chunk_classes) = handle.await {
                all_class_names.extend(chunk_classes);
            }
        }

        debug!(
            "Resolved {} external class names for project {:?}",
            all_class_names.len(),
            project_path
        );
        Ok(all_class_names)
    }
}
