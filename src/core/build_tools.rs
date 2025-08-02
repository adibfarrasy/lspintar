use anyhow::Result;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::process::Command;
use tracing::debug;

#[derive(Debug, Clone)]
pub enum BuildTool {
    Gradle,
}

pub fn detect_build_tool(project_root: &Path) -> Option<BuildTool> {
    if project_root.join("build.gradle").exists() || project_root.join("build.gradle.kts").exists()
    {
        return Some(BuildTool::Gradle);
    }

    None
}

pub async fn parse_settings_gradle(project_root: &PathBuf) -> Result<HashMap<String, PathBuf>> {
    let settings_file = project_root.join("settings.gradle");
    if !settings_file.exists() {
        // No settings.gradle means single project, return empty map
        return Ok(HashMap::new());
    }

    let content = tokio::fs::read_to_string(&settings_file).await?;
    let mut project_map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Handle include syntax: include ':module-a', ':my-project'
        if line.starts_with("include ") {
            let projects_str = line.strip_prefix("include ").unwrap();

            for project_ref in projects_str.split(',') {
                let project_name = project_ref.trim().trim_matches(['\'', '"', ' ']);

                if project_ref.starts_with(':') {
                    // Convert ':my-project' -> 'my-project', ':shared:common' -> 'shared/common'
                    let path_str = project_ref.strip_prefix(':').unwrap().replace(':', "/");
                    project_map.insert(project_name.to_string(), project_root.join(path_str));
                } else {
                    continue;
                };
            }
        }
    }

    Ok(project_map)
}

#[tracing::instrument(skip_all)]
pub async fn execute_gradle_dependencies(
    project_root: &PathBuf,
) -> Result<GradleDependenciesResult> {
    let gradle_command = if project_root.join("gradlew").exists() {
        "./gradlew"
    } else if project_root.join("gradlew.bat").exists() {
        "./gradlew.bat"
    } else {
        "gradle"
    };

    let mut results = GradleDependenciesResult::new();

    for config in &["compileClasspath", "testCompileClasspath"] {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            Command::new(gradle_command)
                .args(&["dependencies", "--configuration", config, "--quiet"])
                .current_dir(project_root)
                .output(),
        )
        .await??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Gradle dependencies failed for {}: {}", config, stderr);
            continue;
        }

        let output_text = String::from_utf8(output.stdout)?;
        results.insert(config.to_string(), output_text);
    }

    if results.is_empty() {
        return Err(anyhow::anyhow!(
            "All gradle dependency configurations failed"
        ));
    }

    Ok(results)
}

#[tracing::instrument(skip_all)]
pub fn parse_gradle_dependencies_output(
    gradle_result: &GradleDependenciesResult,
) -> Result<ParsedGradleDependencies> {
    let mut external_deps = Vec::new();
    let mut project_deps = Vec::new();

    // Parse both compile and test configurations
    for (config_name, output) in &gradle_result.configurations {
        debug!("Parsing {} configuration", config_name);

        for line in output.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() || is_configuration_header(trimmed) {
                continue;
            }

            // Parse project dependencies: "+--- project :my-project"
            if let Some(project_ref) = extract_project_dependency(trimmed) {
                if !project_deps.contains(&project_ref) {
                    project_deps.push(project_ref);
                }
            }

            // Parse external dependencies: "+--- org.springframework:spring-core:5.3.21 -> 5.3.23"
            if let Some(external_dep) = extract_external_dependency(trimmed) {
                if !external_deps.iter().any(|existing: &ExternalDependency| {
                    existing.group == external_dep.group
                        && existing.artifact == external_dep.artifact
                }) {
                    external_deps.push(external_dep);
                }
            }
        }
    }

    Ok(ParsedGradleDependencies {
        external_dependencies: external_deps,
        project_dependencies: project_deps,
    })
}

fn is_configuration_header(line: &str) -> bool {
    line.contains(" - ")
        && (line.contains("compileClasspath")
            || line.contains("testCompileClasspath")
            || line.contains("runtimeClasspath"))
}

fn extract_project_dependency(line: &str) -> Option<String> {
    // Match lines like: "+--- project :my-project" or "\\--- project :other-module"
    if line.contains("project :") {
        if let Some(start) = line.find("project :") {
            let project_part = &line[start + "project ".len()..];
            let project_ref = project_part.split_whitespace().next()?;
            return Some(project_ref.to_string());
        }
    }
    None
}

fn extract_external_dependency(line: &str) -> Option<ExternalDependency> {
    let cleaned = remove_tree_characters(line);

    // group:artifact:version pattern
    let parts: Vec<&str> = cleaned.split(':').collect();
    if parts.len() < 3 {
        return None;
    }

    let group = parts[0].trim().to_string();
    let artifact = parts[1].trim().to_string();
    let version_part = parts[2].trim();

    if let Some(arrow_pos) = version_part.find(" -> ") {
        // Handle conflict resolution: "5.3.21 -> 5.3.23"
        let resolved_version = version_part[arrow_pos + 4..].trim().to_string();

        Some(ExternalDependency {
            group,
            artifact,
            version: resolved_version,
        })
    } else {
        let version = version_part.split_whitespace().next()?.to_string();

        Some(ExternalDependency {
            group,
            artifact,
            version,
        })
    }
}

fn remove_tree_characters(line: &str) -> String {
    line.chars()
        .skip_while(|&c| matches!(c, '+' | '-' | '\\' | '|' | ' '))
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Debug)]
pub struct GradleDependenciesResult {
    pub configurations: HashMap<String, String>,
}

impl GradleDependenciesResult {
    pub fn new() -> Self {
        Self {
            configurations: HashMap::new(),
        }
    }

    pub fn insert(&mut self, config: String, output: String) {
        self.configurations.insert(config, output);
    }

    pub fn is_empty(&self) -> bool {
        self.configurations.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ParsedGradleDependencies {
    pub external_dependencies: Vec<ExternalDependency>,
    pub project_dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
}
