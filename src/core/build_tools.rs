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

use super::{
    constants::{GROOVY_PARSER, JAVA_PARSER},
    dependency_cache::{external::SourceFileInfo, DependencyCache},
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
        debug!("settings.gradle not found");
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

            debug!("Found project: '{}' -> {:?}", project_name, project_path);
            project_map.insert(project_name, project_path);
        } else if !project_ref.is_empty() {
            let project_name = project_ref.to_string().replace(':', "/");
            let project_path = project_root.join(&project_name);

            debug!("Found project: '{}' -> {:?}", project_name, project_path);
            project_map.insert(project_name, project_path);
        }
    }
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

        let duration = start.elapsed();
        debug!(
            "`gradle :{:#?}:dependencies --configuration {} --quiet` command took: {:?}",
            project_root, config, duration
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                "`gradle :{:#?}:dependencies --configuration {} --quiet` command failed: {:?}",
                project_root, config, stderr
            );
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

#[derive(Debug, Clone)]
pub struct ExternalDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

#[tracing::instrument(skip_all)]
pub fn find_jar_in_gradle_cache(dep: &ExternalDependency) -> Option<PathBuf> {
    let cache_base = get_gradle_cache_base()?;

    // Gradle cache structure: group/artifact/version/hash/artifact-version.jar
    let group_path = dep.group.replace('.', "/");
    let artifact_dir = cache_base
        .join(&group_path)
        .join(&dep.artifact)
        .join(&dep.version);

    if !artifact_dir.exists() {
        debug!("Gradle cache directory not found: {:?}", artifact_dir);
        return None;
    }

    // There should be only one hash directory
    let mut read_dir = fs::read_dir(&artifact_dir).ok()?;
    while let Some(entry) = read_dir.next() {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            let jar_name = format!("{}-{}.jar", dep.artifact, dep.version);
            let jar_path = entry.path().join(&jar_name);

            if jar_path.exists() {
                return Some(jar_path);
            }
        }
    }

    debug!(
        "JAR not found for dependency: {}:{}:{}",
        dep.group, dep.artifact, dep.version
    );
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

        // Only process .class files, skip inner classes and test classes
        if file_name.ends_with(".class") && should_index_class(file_name) {
            if let Some(class_name) = class_path_to_name(file_name) {
                class_names.insert(class_name);
            }
        }
    }

    debug!(
        "Extracted {} classes from JAR: {:?}",
        class_names.len(),
        jar_path
    );
    Ok(class_names)
}

fn get_gradle_cache_base() -> Option<PathBuf> {
    match std::env::var("GRADLE_USER_HOME") {
        Ok(cache_path) => Some(PathBuf::from(cache_path).join("caches/modules-2/files-2.1")),
        _ => dirs::home_dir().map(|home| home.join(".gradle/caches/modules-2/files-2.1")),
    }
}

fn should_index_class(class_path: &str) -> bool {
    // Skip inner classes (contain $), test classes, and common build artifacts
    if class_path.contains('$') {
        return false;
    }

    if class_path.contains("/test/") || class_path.contains("/tests/") {
        return false;
    }

    // Skip common build/framework artifacts that aren't user-accessible
    const SKIP_PACKAGES: &[&str] = &["META-INF/", "WEB-INF/", "org/gradle/", "org/apache/maven/"];

    !SKIP_PACKAGES
        .iter()
        .any(|skip| class_path.starts_with(skip))
}

fn class_path_to_name(class_path: &str) -> Option<String> {
    // Convert "com/example/MyClass.class" to "com.example.MyClass"
    let without_extension = class_path.strip_suffix(".class")?;
    Some(without_extension.replace('/', "."))
}

#[tracing::instrument(skip_all)]
pub fn index_jar_sources(
    jar_path: &PathBuf,
    project_path: &PathBuf,
    cache: &Arc<DependencyCache>,
    class_names: &HashSet<String>,
) -> Result<()> {
    let jar_data = fs::read(jar_path)?;

    let jar_path = jar_path.clone();
    let project_path = project_path.clone();
    let cache = cache.clone();
    let class_names = class_names.clone();

    let cursor = Cursor::new(jar_data);
    let mut archive = ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        if !(file_name.ends_with(".java") || file_name.ends_with(".groovy")) {
            continue;
        }

        let class_name = file_name
            .split('/')
            .last()
            .unwrap()
            .trim_end_matches(".java")
            .trim_end_matches(".groovy");

        if !class_names.contains(class_name) {
            continue;
        }

        let mut content = String::new();
        if file.read_to_string(&mut content).is_err() {
            continue;
        }

        if let Err(e) = parse_and_cache_project_external(
            class_name,
            &project_path,
            jar_path.clone(),
            Some(file_name.clone()),
            content,
            &cache,
        ) {
            debug!("Failed to parse external source {}: {}", class_name, e);
        }
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
fn parse_and_cache_project_external(
    class_name: &str,
    project_path: &PathBuf,
    source_path: PathBuf,
    zip_internal_path: Option<String>,
    content: String,
    cache: &DependencyCache,
) -> Result<()> {
    let language = if source_path.extension().and_then(|s| s.to_str()) == Some("groovy")
        || zip_internal_path
            .as_ref()
            .map(|p| p.ends_with(".groovy"))
            .unwrap_or(false)
    {
        GROOVY_PARSER.get_or_init(|| tree_sitter_groovy::language())
    } else {
        JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into())
    };

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(language)
        .with_context(|| "Failed to set parser language")?;

    let tree = parser
        .parse(&content, None)
        .with_context(|| format!("Failed to parse source file: {:?}", source_path))?;

    let external_info = SourceFileInfo {
        source_path,
        zip_internal_path,
        tree,
        content,
    };

    let project_key = (project_path.clone(), class_name.to_string());
    cache
        .project_external_infos
        .insert(project_key, external_info);

    Ok(())
}
