use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::process::Command;
use tracing::debug;
use zip::ZipArchive;

use crate::{lsp_error, lsp_info};

use super::{
    constants::GRADLE_CACHE_DIR,
    dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
    state_manager::get_global,
};

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

#[tracing::instrument(skip_all)]
pub async fn parse_settings_gradle(project_root: &PathBuf) -> Result<HashMap<String, PathBuf>> {
    let settings_file = project_root.join("settings.gradle");
    if !settings_file.exists() {
        return Ok(HashMap::new());
    }

    let content = tokio::fs::read_to_string(&settings_file).await?;
    let mut project_map = HashMap::new();

    let mut in_include_block = false;
    let mut include_content = String::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments
        if line.starts_with("//") || line.starts_with("/*") {
            continue;
        }

        if line.starts_with("include ") {
            in_include_block = true;
            include_content = line.strip_prefix("include ").unwrap().to_string();

            // Check if this is a single-line include (no opening parenthesis or ends with quote)
            if !include_content.contains('(') || include_content.trim_end().ends_with(['\'', '"']) {
                parse_include_content(&include_content, project_root, &mut project_map);
                in_include_block = false;
                include_content.clear();
            }
        } else if in_include_block {
            include_content.push(' ');
            include_content.push_str(line);

            // Check if this line ends the include block (closing parenthesis or ends with quote)
            if line.contains(')') || line.trim_end().ends_with(['\'', '"']) {
                parse_include_content(&include_content, project_root, &mut project_map);
                in_include_block = false;
                include_content.clear();
            }
        }
    }

    Ok(project_map)
}

fn parse_include_content(
    content: &str,
    project_root: &PathBuf,
    project_map: &mut HashMap<String, PathBuf>,
) {
    let content = content.trim_matches(['(', ')', ' ']);

    for project_ref in content.split(',') {
        let project_ref = project_ref.trim().trim_matches(['\'', '"', ' ']);

        if let Some(project_name_with_colons) = project_ref.strip_prefix(':') {
            let project_name = project_name_with_colons.to_string().replace(':', "/");
            let project_path = project_root.join(&project_name);

            project_map.insert(project_name, project_path);
        } else if !project_ref.is_empty() {
            let project_name = project_ref.to_string().replace(':', "/");
            let project_path = project_root.join(&project_name);

            project_map.insert(project_name, project_path);
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn run_gradle_build(project_root: &PathBuf) -> anyhow::Result<()> {
    let gradle_command = if project_root.join("gradlew").exists() {
        "./gradlew"
    } else if project_root.join("gradlew.bat").exists() {
        "./gradlew.bat"
    } else {
        "gradle"
    };

    lsp_info!("Running gradle build...");
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        Command::new(gradle_command)
            .args(&["build", "--quiet", "--parallel"])
            .current_dir(project_root)
            .output(),
    )
    .await
    .context("Gradle build timed out after 5 minutes")??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);


        return Err(anyhow::anyhow!("Gradle build failed. See logs for detail."));
    }

    Ok(())
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

    let start = tokio::time::Instant::now();

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
            continue;
        }

        let output_text = String::from_utf8(output.stdout)?;
        results.insert(config.to_string(), output_text);
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
    for (_, output) in &gradle_result.configurations {
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
            let project_part = &line[start + "project :".len()..];
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

#[tracing::instrument(skip_all)]
pub fn find_sources_jar_in_gradle_cache(dep: &ExternalDependency) -> Option<PathBuf> {
    let cache_base = get_gradle_cache_base()?;

    let mut artifact_dir = cache_base
        .join(&dep.group)
        .join(&dep.artifact)
        .join(&dep.version);

    if !artifact_dir.exists() {
        let group_path = dep.group.replace('.', "/");

        artifact_dir = cache_base
            .join(&group_path)
            .join(&dep.artifact)
            .join(&dep.version);

        if !artifact_dir.exists() {
            return None;
        }
    }

    // There should be only one hash directory
    let mut read_dir = fs::read_dir(&artifact_dir).ok()?;
    while let Some(entry) = read_dir.next() {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            let jar_name = format!("{}-{}-sources.jar", dep.artifact, dep.version);
            let jar_path = entry.path().join(&jar_name);

            if jar_path.exists() {
                return Some(jar_path);
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
pub fn extract_class_names_from_jar(jar_path: &PathBuf) -> Result<HashSet<String>> {
    let jar_data = fs::read(jar_path)?;

    let jar_path = jar_path.clone();
    let cursor = Cursor::new(jar_data);
    let mut archive = ZipArchive::new(cursor)?;
    let mut class_names = HashSet::new();

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let file_name = file.name();

        if (file_name.ends_with(".java") || file_name.ends_with(".groovy"))
            && should_index_source_file(file_name)
        {
            if let Some(class_name) = source_path_to_class_name(file_name) {
                class_names.insert(class_name);
            }
        }
    }

    Ok(class_names)
}

fn should_index_source_file(file_path: &str) -> bool {
    // Skip inner classes (contain $), test classes, and common build artifacts
    if file_path.contains('$') {
        return false;
    }

    if file_path.contains("/test/") || file_path.contains("/tests/") {
        return false;
    }

    // Skip common build/framework artifacts that aren't user-accessible
    const SKIP_PACKAGES: &[&str] = &["META-INF/", "WEB-INF/", "org/gradle/", "org/apache/maven/"];

    !SKIP_PACKAGES.iter().any(|skip| file_path.starts_with(skip))
}

fn source_path_to_class_name(source_path: &str) -> Option<String> {
    // Convert "com/example/MyClass.java" or "com/example/MyClass.groovy" to "com.example.MyClass"
    let without_extension = source_path
        .strip_suffix(".java")
        .or_else(|| source_path.strip_suffix(".groovy"))?;

    Some(without_extension.replace('/', "."))
}

pub fn get_gradle_cache_base() -> Option<PathBuf> {
    // 0. User-defined path
    if let Some(gradle_cache_value) = get_global(GRADLE_CACHE_DIR) {
        if let Some(dir_str) = gradle_cache_value.as_str() {
            let path = PathBuf::from(dir_str);
            if path.exists() {
                return Some(path);
            }
        }
    }

    // 1. Explicit GRADLE_USER_HOME
    if let Ok(gradle_user_home) = std::env::var("GRADLE_USER_HOME") {
        let path = PathBuf::from(gradle_user_home).join("caches/modules-2/files-2.1");
        if path.exists() {
            return Some(path);
        }
    }

    // 2. SYSTEM: GRADLE_HOME
    if let Ok(gradle_home) = std::env::var("GRADLE_HOME") {
        let path = PathBuf::from(gradle_home).join("caches/modules-2/files-2.1");
        if path.exists() {
            return Some(path);
        }
    }

    // 3. FALLBACK: Default user cache
    dirs::home_dir()
        .map(|home| home.join(".gradle/caches/modules-2/files-2.1"))
        .filter(|path| path.exists())
}

#[tracing::instrument(skip_all)]
pub fn index_jar_sources(
    jar_path: &PathBuf,
    project_path: &PathBuf,
    cache: Arc<DependencyCache>,
    class_fqn_names: &HashSet<String>,
    dependency: &ExternalDependency,
) -> Result<()> {
    let jar_data = fs::read(jar_path)?;

    let cursor = Cursor::new(jar_data);
    let mut archive = ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        if !(file_name.ends_with(".java")
            || file_name.ends_with(".groovy")
            || should_index_source_file(&file_name))
        {
            continue;
        }

        let class_name = file_name
            .split('/')
            .last()
            .unwrap()
            .trim_end_matches(".java")
            .trim_end_matches(".groovy");

        let mut content = String::new();
        if file.read_to_string(&mut content).is_err() {
            continue;
        }

        let package_name = extract_package_name(&content);

        if package_name.is_none() {
            continue;
        }

        if !class_fqn_names.contains(&format!("{}.{}", package_name.unwrap(), class_name)) {
            continue;
        }

        let _ = parse_and_cache_project_external(
            class_name,
            project_path,
            jar_path.clone(),
            Some(file_name.clone()),
            dependency,
            cache.clone(),
        );
    }

    Ok(())
}

fn extract_package_name(content: &str) -> Option<String> {
    // NOTE: use any reasonable number to get the first few lines
    for line in content.lines().take(50) {
        let line = line.trim();

        if line.starts_with("package ") {
            let package_part = line
                .strip_prefix("package ")?
                .trim()
                .trim_end_matches(';')
                .trim();

            let package_name = package_part.split_whitespace().next()?.to_string();

            return Some(package_name);
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn parse_and_cache_project_external(
    class_name: &str,
    project_path: &PathBuf,
    jar_path: PathBuf,
    zip_internal_path: Option<String>,
    dependency: &ExternalDependency,
    cache: Arc<DependencyCache>,
) -> Result<()> {
    let external_info = SourceFileInfo::new(jar_path, zip_internal_path, Some(dependency.clone()));

    let project_key = (project_path.clone(), class_name.to_string());

    cache
        .project_external_infos
        .insert(project_key, external_info);

    Ok(())
}
