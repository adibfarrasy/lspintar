use std::{io::Read, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use tracing::debug;
use tree_sitter::Tree;
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::{
    core::{build_tools::BuildTool, utils::create_parser_for_language},
    languages::groovy::constants::GROOVY_DEFAULT_IMPORTS,
};

use super::DependencyCache;

#[derive(Debug, Clone)]
pub struct SourceFileInfo {
    pub source_path: PathBuf,
    pub zip_internal_path: Option<String>, // e.g. "java/lang/String.java"
    pub tree: Tree,
    pub content: String,
}

pub struct DependencyResolver {
    dependency_paths: Vec<PathBuf>,
}

impl DependencyResolver {
    pub fn new(build_tool: &BuildTool) -> Self {
        let mut dependency_paths = Vec::new();

        match build_tool {
            BuildTool::Gradle => dependency_paths.extend(get_gradle_cache()),
            BuildTool::Maven => dependency_paths.extend(get_maven_local_repo()),
        };

        let java_home = std::env::var("JAVA_HOME").ok().map(PathBuf::from);
        let groovy_home = std::env::var("GROOVY_HOME").ok().map(PathBuf::from);

        let paths: Vec<PathBuf> = GROOVY_DEFAULT_IMPORTS
            .iter()
            .map(|i| find_package_source_directory(i, &java_home, &groovy_home))
            .filter_map(|result| result.ok())
            .collect();

        // eagerly load groovy imports, if any
        dependency_paths.extend_from_slice(&paths);

        Self { dependency_paths }
    }

    #[tracing::instrument(skip_all)]
    pub async fn index_external_dependencies(&self, cache: &DependencyCache) -> Result<()> {
        let futures: Vec<_> = self
            .dependency_paths
            .iter()
            .map(|import_path| self.load_package_classes(import_path, cache))
            .collect();

        futures::future::try_join_all(futures).await?;

        debug!("external dependencies initialized.");

        Ok(())
    }

    #[tracing::instrument(skip(self, cache))]
    async fn load_package_classes(
        &self,
        source_path: &PathBuf,
        cache: &DependencyCache,
    ) -> Result<()> {
        let source_files = WalkDir::new(source_path)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext == "java" || ext == "groovy" || ext == "zip")
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        for entry in source_files {
            if entry.path().extension().unwrap().to_str() == Some("zip") {
                self.load_from_zip(&entry.path().to_path_buf(), cache)
                    .await?;
            } else {
                let file_path = entry.path();
                let content = tokio::fs::read_to_string(&file_path).await?;

                if let Some(class_name) = file_path.file_stem().and_then(|s| s.to_str()) {
                    self.parse_and_cache_external(
                        class_name,
                        file_path.to_path_buf(),
                        None,
                        content,
                        cache,
                    )
                    .await
                    .with_context(|| {
                        let err_text = format!("Failed to parse external classes: {}", class_name);
                        debug!(err_text);
                        anyhow!(err_text)
                    })?;
                }
            }
        }

        Ok(())
    }

    async fn load_from_zip(&self, zip_path: &PathBuf, cache: &DependencyCache) -> Result<()> {
        let file = std::fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i)?;
            let file_name = zip_file.name().to_string();

            if file_name.ends_with(".java") || file_name.ends_with(".groovy") {
                let class_name = file_name
                    .split('/')
                    .last()
                    .unwrap()
                    .trim_end_matches(".java")
                    .trim_end_matches(".groovy");

                let mut content = String::new();
                zip_file.read_to_string(&mut content)?;

                self.parse_and_cache_external(
                    class_name,
                    zip_path.clone(),
                    Some(file_name.clone()),
                    content,
                    cache,
                )
                .await?;
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn parse_and_cache_external(
        &self,
        class_name: &str,
        source_path: PathBuf,
        zip_internal_path: Option<String>,
        content: String,
        cache: &DependencyCache,
    ) -> Result<()> {
        let language = if source_path.extension().and_then(|s| s.to_str()) == Some("groovy") {
            "groovy"
        } else {
            "java"
        };

        let mut parser = create_parser_for_language(language).with_context(|| {
            debug!("Failed to create parser for {language}");
            "Failed to create parser"
        })?;

        let tree = parser.parse(&content, None).with_context(|| {
            debug!("Failed to parse source file {:#?}", source_path);
            "Failed to parse source file"
        })?;

        let external_info = SourceFileInfo {
            source_path,
            zip_internal_path,
            tree,
            content,
        };

        cache
            .external_infos
            .insert(class_name.to_string(), external_info);

        Ok(())
    }
}

fn get_maven_local_repo() -> Option<PathBuf> {
    std::env::var("M2_REPO")
        .or_else(|_| std::env::var("maven.repo.local"))
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|home| home.join(".m2/repository")))
}

fn get_gradle_cache() -> Option<PathBuf> {
    if let Ok(cache_path) = std::env::var("GRADLE_USER_HOME") {
        return Some(PathBuf::from(cache_path).join("caches"));
    }

    dirs::home_dir().map(|home| home.join(".gradle/caches"))
}

#[tracing::instrument(skip_all)]
fn find_package_source_directory(
    package: &str,
    java_home: &Option<PathBuf>,
    groovy_home: &Option<PathBuf>,
) -> Result<PathBuf> {
    let package_path = package.replace(".*", "").replace('.', "/");

    if package_path.starts_with("java") {
        if let Some(java_home) = java_home {
            let candidates = [
                java_home.join("src").join(&package_path), // OpenJDK layout
                java_home.join("lib").join("src").join(&package_path), // Some distributions
            ];

            for candidate in &candidates {
                if candidate.exists() {
                    return Ok(candidate.clone());
                }
            }

            let src_zip = java_home.join("src.zip");
            if src_zip.exists() {
                let classes = find_classes_in_zip(&src_zip, &package_path)?;

                debug!(
                    "Found {} classes in src.zip for package {}",
                    classes.len(),
                    package
                );

                if !classes.is_empty() {
                    return Ok(src_zip);
                }
            }
        }
    }

    if package_path.starts_with("groovy") {
        if let Some(groovy_home) = groovy_home {
            let candidates = [
                groovy_home.join("src/main/java").join(&package_path),
                groovy_home.join("src/src/main/java").join(&package_path),
            ];

            for candidate in &candidates {
                if candidate.exists() {
                    return Ok(candidate.clone());
                }
            }
        }
    }

    Err(anyhow!(
        "Could not find source directory for package: {}",
        package
    ))
}

fn find_classes_in_zip(zip_path: &PathBuf, package: &str) -> Result<Vec<String>> {
    let file = std::fs::File::open(zip_path)?;
    let archive = ZipArchive::new(file)?;

    let package_prefix = format!("{}/", package.replace('.', "/"));

    let classes: Vec<String> = archive
        .file_names()
        .filter(|name| {
            // Same level only
            name.starts_with(&package_prefix)
                && (name.ends_with(".java") || name.ends_with(".groovy"))
                && name.matches('/').count() == package_prefix.matches('/').count()
        })
        .map(|name| {
            name.split('/')
                .last()
                .unwrap()
                .trim_end_matches(".java")
                .trim_end_matches(".groovy")
                .to_string()
        })
        .collect();

    Ok(classes)
}

fn try_find_package_name(content: &str) -> Option<String> {
    content
        .lines()
        .find(|line| line.trim_start().starts_with("package "))
        .and_then(|line| {
            line.trim()
                .strip_prefix("package ")?
                .trim_end_matches(';')
                .trim()
                .split_whitespace()
                .next()
                .map(|s| s.to_string())
        })
}
