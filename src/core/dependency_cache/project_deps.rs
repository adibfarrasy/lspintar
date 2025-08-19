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
        debug!("ProjectMapper: index_project_dependencies called for: {:?} with build tool: {:?}", project_root, self.build_tool);
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
                debug!("ProjectMapper: Calling index_project_dependencies_gradle");
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
        debug!("ProjectMapper: index_project_dependencies_gradle starting for: {:?}", project_root);
        let project_map = parse_settings_gradle(&project_root).await?;
        debug!("ProjectMapper: Parsed settings.gradle, found {} projects", project_map.len());

        // Synchronous dependency resolution
        debug!("ProjectMapper: Executing gradle dependencies for all projects synchronously");
        let all_gradle_results = self.execute_gradle_dependencies_synchronous(&project_root, &project_map).await?;
        debug!("ProjectMapper: Got gradle results for {} projects", all_gradle_results.len());

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
                    if let Some(jar_path) = find_sources_jar_in_gradle_cache(&dep) {
                        if let Ok(classes) = extract_class_names_from_jar(&jar_path) {
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
        debug!("ProjectMapper: Processing root project");
        match execute_gradle_dependencies(project_root).await {
            Ok(gradle_result) => {
                if !gradle_result.is_empty() {
                    debug!("ProjectMapper: Root project succeeded with {} configurations", gradle_result.configurations.len());
                    all_gradle_results.insert("".to_string(), gradle_result);
                } else {
                    debug!("ProjectMapper: Root project had empty result");
                }
            }
            Err(e) => {
                debug!("ProjectMapper: Root project failed with error: {}", e);
                debug!("ProjectMapper: This may indicate a Gradle configuration issue in the root project");
            }
        }
        
        // Process each subproject sequentially
        for (project_name, project_path) in project_map {
            debug!("ProjectMapper: Processing project '{}' at {:?}", project_name, project_path);
            match execute_gradle_dependencies(project_path).await {
                Ok(gradle_result) => {
                    if !gradle_result.is_empty() {
                        debug!("ProjectMapper: Project '{}' succeeded with {} configurations", project_name, gradle_result.configurations.len());
                        all_gradle_results.insert(project_name.clone(), gradle_result);
                    } else {
                        debug!("ProjectMapper: Project '{}' had empty result", project_name);
                    }
                }
                Err(e) => {
                    debug!("ProjectMapper: Project '{}' at {:?} failed with error: {}", project_name, project_path, e);
                    debug!("ProjectMapper: This may indicate a Gradle configuration issue in project '{}'", project_name);
                }
            }
        }
        
        if all_gradle_results.is_empty() {
            return Err(anyhow!("Failed to resolve dependencies for any project"));
        }
        
        debug!("ProjectMapper: Synchronous execution completed with {} successful projects", all_gradle_results.len());
        Ok(all_gradle_results)
    }

    /// Optimized parallel Gradle dependency resolution with batching and resource management
    #[tracing::instrument(skip_all)]
    async fn execute_gradle_dependencies_optimized(
        &self,
        project_root: &PathBuf,
        project_map: &HashMap<String, PathBuf>,
    ) -> Result<HashMap<String, GradleDependenciesResult>> {
        use tokio::sync::Semaphore;
        use std::sync::Arc;

        // Strategy 1: Try single multi-project command first (fastest if it works)
        // TEMPORARILY DISABLED FOR DEBUGGING - the single command doesn't parse multi-project output correctly
        debug!("ProjectMapper: Skipping single command approach to use parallel debugging");
        // if let Ok(result) = self.try_single_gradle_command(project_root).await {
        //     return Ok(result);
        // }

        // Strategy 2: Controlled parallel execution with resource limits
        let max_concurrent = std::cmp::min(3, std::cmp::max(1, num_cpus::get() / 2)); // Conservative limit
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        
        let mut tasks = Vec::new();
        let mut all_projects = vec![("".to_string(), project_root.clone())];
        
        // Add all subprojects
        for (name, path) in project_map {
            all_projects.push((name.clone(), path.clone()));
        }
        
        debug!("ProjectMapper: Will attempt Gradle execution for {} projects:", all_projects.len());
        for (name, path) in &all_projects {
            debug!("  Project '{}' at path: {:?}", name, path);
        }

        // Strategy 3: Batch execution - process projects in small groups
        for batch in all_projects.chunks(max_concurrent) {
            let mut batch_tasks = Vec::new();
            
            for (project_name, project_path) in batch {
                let project_name = project_name.clone();
                let project_path = project_path.clone();
                let semaphore = semaphore.clone();
                
                let task = tokio::spawn(async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    
                    // Add small delay to reduce resource contention
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    
                    debug!("ProjectMapper: Attempting Gradle execution for project '{}' at {:?}", project_name, project_path);
                    let result = execute_gradle_dependencies(&project_path).await;
                    match &result {
                        Ok(gradle_result) => {
                            debug!("ProjectMapper: Gradle SUCCESS for project '{}' - got {} configurations", project_name, gradle_result.configurations.len());
                        }
                        Err(e) => {
                            debug!("ProjectMapper: Gradle FAILED for project '{}': {}", project_name, e);
                        }
                    }
                    (project_name, result)
                });
                
                batch_tasks.push(task);
            }
            
            // Wait for current batch to complete before starting next batch
            for task in batch_tasks {
                tasks.push(task);
            }
            
            // Small delay between batches to let Gradle daemon settle
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        // Collect all results
        let mut all_gradle_results = HashMap::new();
        debug!("ProjectMapper: Collecting results from {} tasks", tasks.len());
        for task in tasks {
            if let Ok((project_name, result)) = task.await {
                match result {
                    Ok(gradle_result) => {
                        if !gradle_result.is_empty() {
                            debug!("ProjectMapper: Adding project '{}' to results (has {} configurations)", project_name, gradle_result.configurations.len());
                            all_gradle_results.insert(project_name, gradle_result);
                        } else {
                            debug!("ProjectMapper: Project '{}' had empty Gradle result, skipping", project_name);
                        }
                    }
                    Err(e) => {
                        debug!("ProjectMapper: Failed to get dependencies for project '{}': {}", project_name, e);
                        // Continue with other projects instead of failing completely
                    }
                }
            } else {
                debug!("ProjectMapper: Task join failed for a project");
            }
        }

        if all_gradle_results.is_empty() {
            return Err(anyhow!("Failed to resolve dependencies for any project"));
        }

        Ok(all_gradle_results)
    }

    /// Try to use a single Gradle command to get all dependencies at once
    async fn try_single_gradle_command(
        &self,
        project_root: &PathBuf,
    ) -> Result<HashMap<String, GradleDependenciesResult>> {
        use tokio::process::Command;

        debug!("ProjectMapper: try_single_gradle_command starting for: {:?}", project_root);

        let gradle_command = if project_root.join("gradlew").exists() {
            "./gradlew"
        } else if project_root.join("gradlew.bat").exists() {
            "./gradlew.bat"
        } else {
            "gradle"
        };

        debug!("ProjectMapper: Using gradle command: {}", gradle_command);

        // Try to get all dependencies with a single command
        // This uses Gradle's ability to run tasks on all subprojects
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(45), // Shorter timeout for this attempt
            Command::new(gradle_command)
                .args(&[
                    "dependencies",
                    "--configuration", "compileClasspath",
                    "--quiet",
                    "--parallel", // Enable Gradle's internal parallelism
                    "--max-workers=4", // Limit Gradle workers
                ])
                .current_dir(project_root)
                .output(),
        )
        .await??;

        if output.status.success() {
            let output_text = String::from_utf8(output.stdout)?;
            
            let mut result = GradleDependenciesResult::new();
            result.insert("compileClasspath".to_string(), output_text);
            
            // For single command, return as root project
            let mut results = HashMap::new();
            results.insert("".to_string(), result);
            return Ok(results);
        } else {
        }

        Err(anyhow!("Single command approach failed"))
    }
}
