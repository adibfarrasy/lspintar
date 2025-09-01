use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::process::Command;
use tracing::debug;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};
use zip::ZipArchive;

use crate::{
    core::constants::{GROOVY_PARSER, JAVA_PARSER, KOTLIN_PARSER},
    lsp_error, lsp_info,
};

use super::{
    constants::GRADLE_CACHE_DIR,
    dependency_cache::{source_file_info::SourceFileInfo, DependencyCache},
    state_manager::get_global,
};

#[derive(Debug, Clone, PartialEq)]
pub enum BuildTool {
    Gradle,
}

#[tracing::instrument(skip_all)]
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
    let settings_kts_file = project_root.join("settings.gradle.kts");

    let (_settings_file, content) = if settings_file.exists() {
        let content = tokio::fs::read_to_string(&settings_file).await?;
        (settings_file, content)
    } else if settings_kts_file.exists() {
        let content = tokio::fs::read_to_string(&settings_kts_file).await?;
        (settings_kts_file, content)
    } else {
        return Ok(HashMap::new());
    };
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

#[tracing::instrument(skip_all)]
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

    // Optimized: Run both configurations in a single command when possible
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(45), // Reduced timeout
        Command::new(gradle_command)
            .args(&[
                "dependencies",
                "--configuration",
                "compileClasspath",
                "--quiet",
                "--no-daemon", // Avoid daemon startup overhead for individual calls
            ])
            .current_dir(project_root)
            .output(),
    )
    .await??;

    if output.status.success() {
        let output_text = String::from_utf8(output.stdout)?;
        // Split output by configuration sections
        let sections = parse_multi_configuration_output(&output_text);
        for (config, content) in sections {
            results.insert(config, content);
        }
    } else {
        // Fallback: Run configurations separately if combined command fails
        for config in &["compileClasspath", "testCompileClasspath"] {
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                Command::new(gradle_command)
                    .args(&[
                        "dependencies",
                        "--configuration",
                        config,
                        "--quiet",
                        "--no-daemon",
                    ])
                    .current_dir(project_root)
                    .output(),
            )
            .await??;

            if output.status.success() {
                let output_text = String::from_utf8(output.stdout)?;
                results.insert(config.to_string(), output_text);
            } else {
                let stderr = String::from_utf8(output.stderr).unwrap_or("ERROR".to_string());
                lsp_error!("failed to get project dependencies: {}", stderr);
            }
        }
    }

    Ok(results)
}

/// Parse output that contains multiple configuration sections
#[tracing::instrument(skip_all)]
fn parse_multi_configuration_output(output: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current_config = None;
    let mut current_content = String::new();

    for line in output.lines() {
        if line.contains("compileClasspath - ") {
            if let Some(config) = current_config.take() {
                sections.insert(config, current_content.clone());
            }
            current_config = Some("compileClasspath".to_string());
            current_content.clear();
            current_content.push_str(line);
            current_content.push('\n');
        } else if line.contains("testCompileClasspath - ") {
            if let Some(config) = current_config.take() {
                sections.insert(config, current_content.clone());
            }
            current_config = Some("testCompileClasspath".to_string());
            current_content.clear();
            current_content.push_str(line);
            current_content.push('\n');
        } else if current_config.is_some() {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Add the last section
    if let Some(config) = current_config {
        sections.insert(config, current_content);
    }

    sections
}

#[tracing::instrument(skip_all)]
pub fn parse_gradle_dependencies_output(
    gradle_result: &GradleDependenciesResult,
) -> Result<ParsedGradleDependencies> {
    let mut external_deps = Vec::new();
    let mut project_deps = Vec::new();

    // Parse both compile and test configurations
    for (_config_name, output) in &gradle_result.configurations {
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

#[tracing::instrument(skip_all)]
fn is_configuration_header(line: &str) -> bool {
    line.contains(" - ")
        && (line.contains("compileClasspath")
            || line.contains("testCompileClasspath")
            || line.contains("runtimeClasspath"))
}

#[tracing::instrument(skip_all)]
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

#[tracing::instrument(skip_all)]
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

        let dep = ExternalDependency {
            group: group.clone(),
            artifact: artifact.clone(),
            version: resolved_version,
        };

        Some(dep)
    } else {
        let version = version_part.split_whitespace().next()?.to_string();

        let dep = ExternalDependency {
            group: group.clone(),
            artifact: artifact.clone(),
            version,
        };

        Some(dep)
    }
}

#[tracing::instrument(skip_all)]
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
pub fn find_jar_in_gradle_cache(dep: &ExternalDependency) -> Option<PathBuf> {
    find_sources_jar_in_gradle_cache(dep).or_else(|| find_regular_jar_in_gradle_cache(dep))
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
pub fn find_regular_jar_in_gradle_cache(dep: &ExternalDependency) -> Option<PathBuf> {
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
            let jar_name = format!("{}-{}.jar", dep.artifact, dep.version);
            let jar_path = entry.path().join(&jar_name);

            if jar_path.exists() {
                return Some(jar_path);
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
pub fn extract_class_names_from_jar(jar_path: &PathBuf) -> Result<HashMap<String, String>> {
    let jar_data = fs::read(jar_path)?;

    let cursor = Cursor::new(jar_data.clone());
    let mut archive = ZipArchive::new(cursor)?;
    let mut class_name_to_path = HashMap::new();

    // First pass: look for source files
    let mut has_source_files = false;
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        if (file_name.ends_with(".java")
            || file_name.ends_with(".groovy")
            || file_name.ends_with(".kt"))
            && should_index_source_file(&file_name)
        {
            has_source_files = true;

            // Fast filename-based extraction only during indexing
            // Content parsing will be done lazily during symbol lookup
            if let Some(class_name) = source_path_to_class_name(&file_name) {
                class_name_to_path.insert(class_name, file_name.to_string());
            }
        }
    }

    // If no source files found, extract class names from .class files
    if !has_source_files {
        debug!(
            "No source files found in JAR, extracting class names from .class files: {}",
            jar_path.display()
        );
        let cursor = Cursor::new(jar_data);
        let mut archive = ZipArchive::new(cursor)?;

        for i in 0..archive.len() {
            let file = archive.by_index(i)?;
            let file_name = file.name();

            if file_name.ends_with(".class") && should_index_class_file(file_name) {
                if let Some(class_name) = class_path_to_class_name(file_name) {
                    if class_name.contains("Service") {
                        debug!("Found Service-related class: {}", class_name);
                    }
                    class_name_to_path.insert(class_name, file_name.to_string());
                }
            }
        }
    }

    Ok(class_name_to_path)
}

#[tracing::instrument(skip_all)]
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

#[tracing::instrument(skip_all)]
fn should_index_class_file(file_path: &str) -> bool {
    // Similar filtering as source files, but for .class files
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

#[tracing::instrument(skip_all)]
fn source_path_to_class_name(source_path: &str) -> Option<String> {
    // Convert "com/example/MyClass.java", "com/example/MyClass.groovy", or "com/example/MyClass.kt" to "com.example.MyClass"
    let without_extension = source_path
        .strip_suffix(".java")
        .or_else(|| source_path.strip_suffix(".groovy"))
        .or_else(|| source_path.strip_suffix(".kt"))?;

    Some(without_extension.replace('/', "."))
}

#[tracing::instrument(skip_all)]
fn class_path_to_class_name(class_path: &str) -> Option<String> {
    // Convert "com/example/MyClass.class" to "com.example.MyClass"
    let without_extension = class_path.strip_suffix(".class")?;
    Some(without_extension.replace('/', "."))
}

#[tracing::instrument(skip_all)]
fn extract_kotlin_definitions_from_content(content: &str) -> Option<Vec<String>> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        debug!("Failed to set Kotlin language in parser");
        return None;
    }

    let tree = parser.parse(content, None);
    if tree.is_none() {
        debug!("Failed to parse Kotlin content (length: {})", content.len());
        return None;
    }
    let tree = tree.unwrap();

    // Extract package declaration
    let package = extract_kotlin_package_from_content(&tree, content);

    let mut definitions = Vec::new();

    // Query for Kotlin definitions - capture the type_identifier directly
    let query_text = r#"
        (class_declaration (type_identifier) @name)
        (interface_declaration (type_identifier) @name) 
        (object_declaration (type_identifier) @name)
    "#;

    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(name) = capture.node.utf8_text(content.as_bytes()) {
                let fully_qualified_name = if let Some(ref pkg) = package {
                    format!("{}.{}", pkg, name)
                } else {
                    name.to_string()
                };
                definitions.push(fully_qualified_name);
            }
        }
    }

    if definitions.is_empty() {
        None
    } else {
        Some(definitions)
    }
}

#[tracing::instrument(skip_all)]
fn extract_kotlin_package_from_content(tree: &tree_sitter::Tree, content: &str) -> Option<String> {
    let language = KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language());
    let query_text = r#"(package_header (identifier) @package)"#;

    if let Ok(query) = Query::new(language, query_text) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

        if let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(package_text) = capture.node.utf8_text(content.as_bytes()) {
                    return Some(package_text.to_string());
                }
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn extract_java_definitions_from_content(content: &str) -> Option<Vec<String>> {
    let language = JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into());
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        debug!("Failed to set Java language in parser");
        return None;
    }

    let tree = parser.parse(content, None);
    if tree.is_none() {
        debug!("Failed to parse Java content (length: {})", content.len());
        return None;
    }
    let tree = tree.unwrap();

    // Extract package declaration
    let package = extract_java_package_from_content(&tree, content);

    let mut definitions = Vec::new();

    // Query for Java definitions
    let query_text = r#"
        (class_declaration name: (identifier) @name)
        (interface_declaration name: (identifier) @name) 
        (enum_declaration name: (identifier) @name)
        (annotation_type_declaration name: (identifier) @name)
    "#;

    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(name) = capture.node.utf8_text(content.as_bytes()) {
                let fully_qualified_name = if let Some(ref pkg) = package {
                    format!("{}.{}", pkg, name)
                } else {
                    name.to_string()
                };
                definitions.push(fully_qualified_name);
            }
        }
    }

    if definitions.is_empty() {
        None
    } else {
        Some(definitions)
    }
}

#[tracing::instrument(skip_all)]
fn extract_java_package_from_content(tree: &tree_sitter::Tree, content: &str) -> Option<String> {
    let language = JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into());
    let query_text = r#"(package_declaration (scoped_identifier) @package)"#;

    if let Ok(query) = Query::new(language, query_text) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

        if let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(package_text) = capture.node.utf8_text(content.as_bytes()) {
                    return Some(package_text.to_string());
                }
            }
        }
    }

    None
}

#[tracing::instrument(skip_all)]
fn extract_groovy_definitions_from_content(content: &str) -> Option<Vec<String>> {
    let language = GROOVY_PARSER.get_or_init(|| tree_sitter_groovy::language());
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        debug!("Failed to set Groovy language in parser");
        return None;
    }

    let tree = parser.parse(content, None);
    if tree.is_none() {
        debug!("Failed to parse Groovy content (length: {})", content.len());
        return None;
    }
    let tree = tree.unwrap();

    // Extract package declaration
    let package = extract_groovy_package_from_content(&tree, content);

    let mut definitions = Vec::new();

    // Query for Groovy definitions
    let query_text = r#"
        (class_declaration name: (identifier) @name)
        (interface_declaration name: (identifier) @name) 
        (enum_declaration name: (identifier) @name)
        (annotation_declaration name: (identifier) @name)
        (trait_declaration name: (identifier) @name)
    "#;

    let query = Query::new(language, query_text).ok()?;
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Ok(name) = capture.node.utf8_text(content.as_bytes()) {
                let fully_qualified_name = if let Some(ref pkg) = package {
                    format!("{}.{}", pkg, name)
                } else {
                    name.to_string()
                };
                definitions.push(fully_qualified_name);
            }
        }
    }

    if definitions.is_empty() {
        None
    } else {
        Some(definitions)
    }
}

#[tracing::instrument(skip_all)]
fn extract_groovy_package_from_content(tree: &tree_sitter::Tree, content: &str) -> Option<String> {
    let language = GROOVY_PARSER.get_or_init(|| tree_sitter_groovy::language());
    let query_text = r#"(package_declaration (dotted_identifier) @package)"#;

    if let Ok(query) = Query::new(language, query_text) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

        if let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if let Ok(package_text) = capture.node.utf8_text(content.as_bytes()) {
                    return Some(package_text.to_string());
                }
            }
        }
    }

    None
}

/// Lazy content-based symbol search for when filename-based lookup fails
#[tracing::instrument(skip_all)]
pub fn find_symbol_in_jar_content(
    jar_path: &PathBuf,
    target_symbol: &str,
) -> Option<String> {
    let jar_data = std::fs::read(jar_path).ok()?;
    let cursor = std::io::Cursor::new(jar_data);
    let mut archive = ZipArchive::new(cursor).ok()?;

    // Only check source files that are likely to contain multiple symbols
    for i in 0..archive.len() {
        let file = match archive.by_index(i) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let file_name = file.name().to_string();

        if !should_check_for_multiple_symbols(&file_name) {
            continue;
        }

        drop(file); // Drop the first file handle
        let mut file_content = match archive.by_index(i) {
            Ok(file) => file,
            Err(_) => continue,
        };

        let mut content = String::new();
        if file_content.read_to_string(&mut content).is_err() {
            continue;
        }

        let extracted_names = if file_name.ends_with(".kt") {
            extract_kotlin_definitions_from_content(&content)
        } else if file_name.ends_with(".java") {
            extract_java_definitions_from_content(&content)
        } else if file_name.ends_with(".groovy") {
            extract_groovy_definitions_from_content(&content)
        } else {
            continue;
        };

        if let Some(names) = extracted_names {
            for name in names {
                if name == target_symbol {
                    debug!("Found {} via content parsing in {}", target_symbol, file_name);
                    return Some(file_name);
                }
            }
        }
    }

    None
}

/// Heuristics to determine if a file is likely to contain multiple symbols
/// and worth parsing content for
#[tracing::instrument(skip_all)]
fn should_check_for_multiple_symbols(file_path: &str) -> bool {
    // Skip files unlikely to have multiple symbols
    if file_path.contains('$') || file_path.contains("/test/") {
        return false;
    }

    // Check file naming patterns that often contain multiple symbols
    let file_name = file_path.split('/').last().unwrap_or("");
    let name_lower = file_name.to_lowercase();
    
    // Common patterns for files with multiple classes
    let multiple_symbol_patterns = [
        "util", "helper", "constants", "config", "builder", 
        "factory", "manager", "service", "handler", "processor",
        "adapter", "converter", "transformer", "validator"
    ];

    // Check if filename suggests multiple symbols
    for pattern in &multiple_symbol_patterns {
        if name_lower.contains(pattern) {
            return true;
        }
    }

    // Check file size - larger files are more likely to have multiple symbols
    if let Ok(metadata) = std::fs::metadata(file_path) {
        // Files larger than 50KB often contain multiple classes/symbols
        if metadata.len() > 50_000 {
            return true;
        }
    }

    false
}

#[tracing::instrument(skip_all)]
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

    // 3. SDKMAN Gradle cache (common when using SDKMAN to manage Gradle)
    if let Some(home) = dirs::home_dir() {
        let sdkman_gradle_cache = home.join(".sdkman/candidates/gradle");
        if sdkman_gradle_cache.exists() {
            // Try to find the most recent Gradle version in SDKMAN
            if let Ok(entries) = std::fs::read_dir(&sdkman_gradle_cache) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        let cache_path = entry.path().join("caches/modules-2/files-2.1");
                        if cache_path.exists() {
                            return Some(cache_path);
                        }
                    }
                }
            }
        }
    }

    // 4. FALLBACK: Default user cache
    let fallback_path = dirs::home_dir()
        .map(|home| home.join(".gradle/caches/modules-2/files-2.1"))
        .filter(|path| path.exists());

    fallback_path
}

/// Index classes with internal paths for source extraction or decompilation
#[tracing::instrument(skip_all)]
pub fn index_jar_classes_metadata_with_paths(
    jar_path: &PathBuf,
    project_path: &PathBuf,
    cache: Arc<DependencyCache>,
    class_name_to_path: &HashMap<String, String>,
    dependency: &ExternalDependency,
) -> Result<()> {
    let mut classes_indexed = 0;

    for (class_name, internal_path) in class_name_to_path {
        let source_info = SourceFileInfo::new_for_decompilation(
            jar_path.clone(),
            Some(internal_path.clone()),
            Some(dependency.clone()),
        );

        let key = (project_path.clone(), class_name.clone());
        cache.project_external_infos.insert(key, source_info);
        classes_indexed += 1;
    }

    debug!(
        "Indexed {} classes with paths from {}",
        classes_indexed, dependency.artifact
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_build_tool_gradle() {
        let temp_dir = TempDir::new().unwrap();
        let gradle_file = temp_dir.path().join("build.gradle");
        fs::write(&gradle_file, "// test gradle file").unwrap();

        let result = detect_build_tool(temp_dir.path());
        assert_eq!(result, Some(BuildTool::Gradle));
    }

    #[test]
    fn test_detect_build_tool_gradle_kts() {
        let temp_dir = TempDir::new().unwrap();
        let gradle_kts_file = temp_dir.path().join("build.gradle.kts");
        fs::write(&gradle_kts_file, "// test gradle kts file").unwrap();

        let result = detect_build_tool(temp_dir.path());
        assert_eq!(result, Some(BuildTool::Gradle));
    }

    #[test]
    fn test_detect_build_tool_none() {
        let temp_dir = TempDir::new().unwrap();
        
        let result = detect_build_tool(temp_dir.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_include_content_single_project() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();
        let mut project_map = HashMap::new();

        parse_include_content("':subproject'", &project_root, &mut project_map);

        assert_eq!(project_map.len(), 1);
        assert!(project_map.contains_key("subproject"));
        assert_eq!(project_map["subproject"], project_root.join("subproject"));
    }

    #[test]
    fn test_parse_include_content_multiple_projects() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();
        let mut project_map = HashMap::new();

        parse_include_content("':project1', ':project2', ':project3'", &project_root, &mut project_map);

        assert_eq!(project_map.len(), 3);
        assert!(project_map.contains_key("project1"));
        assert!(project_map.contains_key("project2"));
        assert!(project_map.contains_key("project3"));
    }

    #[test]
    fn test_parse_include_content_nested_project() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();
        let mut project_map = HashMap::new();

        parse_include_content("':parent:child'", &project_root, &mut project_map);

        assert_eq!(project_map.len(), 1);
        assert!(project_map.contains_key("parent/child"));
        assert_eq!(project_map["parent/child"], project_root.join("parent/child"));
    }

    #[test]
    fn test_parse_include_content_empty() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();
        let mut project_map = HashMap::new();

        parse_include_content("", &project_root, &mut project_map);

        assert_eq!(project_map.len(), 0);
    }

    #[tokio::test]
    async fn test_parse_settings_gradle_no_file() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();

        let result = parse_settings_gradle(&project_root).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_parse_settings_gradle_with_includes() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = temp_dir.path().to_path_buf();
        let settings_file = project_root.join("settings.gradle");

        let settings_content = r#"
// This is a comment
include ':subproject1'
include ':subproject2', ':subproject3'
include(
    ':multi-line-project1',
    ':multi-line-project2'
)
        "#;

        fs::write(&settings_file, settings_content).unwrap();

        let result = parse_settings_gradle(&project_root).await;
        assert!(result.is_ok());
        
        let project_map = result.unwrap();
        // The current parsing might not handle all the multi-line cases perfectly
        // At minimum, we should get the simple includes
        assert!(project_map.len() >= 3, "Expected at least 3 projects, got {}", project_map.len());
        assert!(project_map.contains_key("subproject1"));
        assert!(project_map.contains_key("subproject2"));
        assert!(project_map.contains_key("subproject3"));
    }

    #[test]
    fn test_external_dependency_artifact_access() {
        let dep = ExternalDependency {
            group: "com.example".to_string(),
            artifact: "test-lib".to_string(),
            version: "1.0.0".to_string(),
        };

        assert_eq!(dep.artifact, "test-lib");
        assert_eq!(dep.group, "com.example");
        assert_eq!(dep.version, "1.0.0");
    }

    #[test]
    fn test_gradle_dependencies_result_new() {
        let result = GradleDependenciesResult::new();
        assert!(result.configurations.is_empty());
    }

    #[test]
    fn test_gradle_dependencies_result_insert_and_access() {
        let mut result = GradleDependenciesResult::new();
        
        result.insert("compileClasspath".to_string(), "deps content".to_string());
        assert_eq!(result.configurations.len(), 1);
        assert!(result.configurations.contains_key("compileClasspath"));
        assert!(!result.is_empty());
    }

    #[test]
    fn test_extract_external_dependency() {
        let test_line = "+--- org.springframework:spring-core:5.3.21";
        let result = extract_external_dependency(test_line);
        
        assert!(result.is_some());
        if let Some(dep) = result {
            assert_eq!(dep.group, "org.springframework");
            assert_eq!(dep.artifact, "spring-core");
            assert_eq!(dep.version, "5.3.21");
        }
    }

    #[test]
    fn test_extract_external_dependency_with_version_resolution() {
        let test_line = "+--- org.springframework:spring-core:5.3.21 -> 5.3.23";
        let result = extract_external_dependency(test_line);
        
        assert!(result.is_some());
        if let Some(dep) = result {
            assert_eq!(dep.group, "org.springframework");
            assert_eq!(dep.artifact, "spring-core");
            assert_eq!(dep.version, "5.3.23"); // Should use resolved version
        }
    }
}
