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
    ExternalDependency,
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
        debug!("Found {} projects in settings.gradle", project_map.len());

        let mut all_gradle_results = HashMap::new();
        for (project_name, subproject_path) in project_map.iter() {
            let result = execute_gradle_dependencies(&subproject_path).await?;

            if result.is_empty() {
                return Ok(());
            }

            all_gradle_results.insert(project_name.to_owned(), result);
        }

        let root_gradle_result = execute_gradle_dependencies(&project_root).await?;
        all_gradle_results.insert("".to_string(), root_gradle_result);

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
                match project_map.get(project_name) {
                    Some(path) => {
                        debug!("Processing project {} -> {:?}", project_name, path);
                        path.clone()
                    }
                    None => {
                        debug!(
                            "Project path not found for {}, available keys: {:?}",
                            project_name,
                            project_map.keys().collect::<Vec<_>>()
                        );
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
                    if let Some(jar_path) = find_sources_jar_in_gradle_cache(&dep) {
                        debug!("Found JAR: {:?}", jar_path);
                        if let Ok(classes) = extract_class_names_from_jar(&jar_path) {
                            debug!("JAR {} has {} classes", jar_path.display(), classes.len());
                            chunk_classes.extend(classes.clone());

                            if let Err(e) = index_jar_sources(
                                &jar_path,
                                &project_path,
                                cache.clone(),
                                &classes,
                                &dep,
                            ) {
                                debug!("JAR source indexing failed for {:?}: {}", jar_path, e);
                            } else {
                                debug!("JAR source indexing completed for {:?}", jar_path);
                            }
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

        debug!(
            "Resolved {} external class names for project {:?}",
            all_class_names.len(),
            project_path
        );
        Ok(all_class_names)
    }
}
