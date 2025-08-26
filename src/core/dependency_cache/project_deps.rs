use anyhow::{anyhow, Result};
use dashmap::DashSet;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tracing::debug;

use crate::core::build_tools::{
    execute_gradle_dependencies, extract_class_names_from_jar, find_sources_jar_in_gradle_cache,
    index_jar_sources, parse_gradle_dependencies_output, parse_settings_gradle, BuildTool,
    ExternalDependency, GradleDependenciesResult,
};

use super::DependencyCache;

#[derive(Debug, Clone)]
pub struct ProjectMetadata {
    // other project roots a project depends on
    pub inter_project_deps: Arc<DashSet<PathBuf>>,

    // External class names available to a project
    pub external_dep_names: Arc<DashSet<String>>,

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

        // Synchronous dependency resolution
        let all_gradle_results = self
            .execute_gradle_dependencies_synchronous(&project_root, &project_map)
            .await?;

        let mut all_parsed_deps = HashMap::new();
        for (project_name, gradle_result) in &all_gradle_results {
            let parsed_deps = parse_gradle_dependencies_output(gradle_result)?;
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
                match project_map.get(project_name) {
                    Some(path) => path.clone(),
                    None => {
                        return Err(anyhow!("Project path not found for {}", project_name));
                    }
                }
            };

            let class_names = self.resolve_and_index_external_dependencies(
                external_deps,
                &current_project_path,
                cache.clone(),
            )?;

            cache
                .project_metadata
                .entry(current_project_path.clone())
                .or_insert_with(|| ProjectMetadata {
                    inter_project_deps: Arc::new(DashSet::new()),
                    external_dep_names: Arc::new(DashSet::new()),
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
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn resolve_and_index_external_dependencies(
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

            let handle = std::thread::spawn(move || {
                let mut chunk_classes = HashSet::new();
                for dep in chunk {
                    debug!("Processing external dependency: {}:{}", dep.group, dep.artifact);
                    if let Some(jar_path) = find_sources_jar_in_gradle_cache(&dep) {
                        debug!("Found sources jar for {}: {:?}", dep.artifact, jar_path);
                        if let Ok(classes) = extract_class_names_from_jar(&jar_path) {
                            if dep.artifact.contains("kotlin") {
                                debug!("Extracted {} classes from kotlin JAR {}: {:?}", classes.len(), dep.artifact, classes.iter().take(10).collect::<Vec<_>>());
                            }
                            chunk_classes.extend(classes.clone());

                            let _ = index_jar_sources(
                                &jar_path,
                                &project_path,
                                cache.clone(),
                                &classes,
                                &dep,
                            );
                        }
                    }
                }
                chunk_classes
            });
            handles.push(handle);
        }

        for handle in handles {
            if let Ok(chunk_classes) = handle.join() {
                all_class_names.extend(chunk_classes);
            }
        }

        Ok(all_class_names)
    }

    /// Synchronous Gradle dependency resolution - more reliable than parallel
    #[tracing::instrument(skip_all)]
    async fn execute_gradle_dependencies_synchronous(
        &self,
        project_root: &PathBuf,
        project_map: &HashMap<String, PathBuf>,
    ) -> Result<HashMap<String, GradleDependenciesResult>> {
        let mut all_gradle_results = HashMap::new();

        // Process root project first
        match execute_gradle_dependencies(project_root).await {
            Ok(gradle_result) => {
                if !gradle_result.is_empty() {
                    all_gradle_results.insert("".to_string(), gradle_result);
                }
            }
            Err(e) => {
                debug!("ProjectMapper: Root project failed with error: {}", e);
            }
        }

        // Process each subproject sequentially
        for (project_name, project_path) in project_map {
            match execute_gradle_dependencies(project_path).await {
                Ok(gradle_result) => {
                    if !gradle_result.is_empty() {
                        all_gradle_results.insert(project_name.clone(), gradle_result);
                    }
                }
                Err(e) => {
                    debug!(
                        "ProjectMapper: Project '{}' at {:?} failed with error: {}",
                        project_name, project_path, e
                    );
                }
            }
        }

        if all_gradle_results.is_empty() {
            return Err(anyhow!("Failed to resolve dependencies for any project"));
        }
        Ok(all_gradle_results)
    }
}
