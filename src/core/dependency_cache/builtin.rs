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
pub struct BuiltinTypeInfo {
    pub source_path: PathBuf,
    pub zip_internal_path: Option<String>, // "java/lang/String.java"
    pub tree: Tree,
    pub content: String,
    pub package: String,
}

pub struct BuiltinResolver {
    java_home: Option<PathBuf>,
    groovy_home: Option<PathBuf>,
    build_tool_home: Option<PathBuf>,
}

impl BuiltinResolver {
    pub fn new(build_tool: &BuildTool) -> Self {
        let build_tool_home = match build_tool {
            BuildTool::Gradle => std::env::var("GRADLE_HOME").ok().map(PathBuf::from),
            BuildTool::Maven => std::env::var("M2_HOME").ok().map(PathBuf::from),
        };

        Self {
            java_home: std::env::var("JAVA_HOME").ok().map(PathBuf::from),
            groovy_home: std::env::var("GROOVY_HOME").ok().map(PathBuf::from),
            build_tool_home,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn initialize_builtins(&self, cache: &DependencyCache) -> Result<()> {
        // TODO: currently assumes it's a groovy project. should check if the imports are necessary.
        let futures: Vec<_> = GROOVY_DEFAULT_IMPORTS
            .iter()
            .map(|import_pattern| self.resolve_import_pattern(import_pattern, cache))
            .collect();

        futures::future::try_join_all(futures).await?;

        debug!("builtins initialized.");

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_import_pattern(
        &self,
        import_pattern: &str,
        cache: &DependencyCache,
    ) -> Result<()> {
        if import_pattern.ends_with(".*") {
            let package = &import_pattern[..import_pattern.len() - 2]; // Remove ".*"
            self.load_package_classes(package, cache).await?;
        } else {
            self.load_specific_class(import_pattern, cache).await?;
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn load_package_classes(&self, package: &str, cache: &DependencyCache) -> Result<()> {
        self.load_classes_filtered(package, None, cache).await
    }

    async fn load_specific_class(
        &self,
        full_class_name: &str,
        cache: &DependencyCache,
    ) -> Result<()> {
        let parts: Vec<&str> = full_class_name.rsplitn(2, '.').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid class name: {}", full_class_name));
        }

        let class_name = parts[0];
        let package = parts[1];

        self.load_classes_filtered(package, Some(class_name), cache)
            .await
    }

    async fn load_classes_filtered(
        &self,
        package: &str,
        specific_class: Option<&str>, // None = all classes, Some = specific class only
        cache: &DependencyCache,
    ) -> Result<()> {
        let source_path = self
            .find_package_source_directory(package)
            .with_context(|| {
                let err_text = format!("Failed to find source directory for package: {}", package);
                debug!(err_text);
                anyhow!(err_text)
            })?;

        if source_path.extension().and_then(|s| s.to_str()) == Some("zip") {
            self.load_from_zip(&source_path, package, specific_class, cache)
                .await?
        } else {
            self.load_from_directory(&source_path, package, specific_class, cache)
                .await?
        }

        // Only cache package mapping for wildcard imports
        if specific_class.is_none() {
            cache
                .builtin_packages
                .insert(format!("{}.*", package), source_path);
        }

        Ok(())
    }

    async fn load_from_directory(
        &self,
        source_dir: &PathBuf,
        package: &str,
        specific_class: Option<&str>,
        cache: &DependencyCache,
    ) -> Result<()> {
        let source_files = WalkDir::new(source_dir)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let path = entry.path();
                let is_source = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext == "java" || ext == "groovy")
                    .unwrap_or(false);

                if let Some(target_class) = specific_class {
                    let class_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    is_source && class_name == target_class
                } else {
                    is_source
                }
            })
            .collect::<Vec<_>>();

        for entry in source_files {
            let file_path = entry.path();
            let content = tokio::fs::read_to_string(&file_path).await?;

            if let Some(class_name) = file_path.file_stem().and_then(|s| s.to_str()) {
                self.parse_and_cache_builtin(
                    class_name,
                    file_path.to_path_buf(),
                    None,
                    content,
                    package,
                    cache,
                )
                .await
                .with_context(|| {
                    let err_text = format!("Failed to parse builtin class: {}", class_name);
                    debug!(err_text);
                    anyhow!(err_text)
                })?;
            }
        }

        Ok(())
    }

    async fn load_from_zip(
        &self,
        zip_path: &PathBuf,
        package: &str,
        specific_class: Option<&str>,
        cache: &DependencyCache,
    ) -> Result<()> {
        let file = std::fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        let package_prefix = format!("{}/", package.replace('.', "/"));

        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i)?;
            let file_name = zip_file.name().to_string();

            if file_name.starts_with(&package_prefix)
                && (file_name.ends_with(".java") || file_name.ends_with(".groovy"))
            {
                let class_name = file_name
                    .split('/')
                    .last()
                    .unwrap()
                    .trim_end_matches(".java")
                    .trim_end_matches(".groovy");

                // Filter to specific class if requested
                if let Some(target_class) = specific_class {
                    if class_name != target_class {
                        continue;
                    }
                }

                let mut content = String::new();
                zip_file.read_to_string(&mut content)?;

                self.parse_and_cache_builtin(
                    class_name,
                    zip_path.clone(),
                    Some(file_name.clone()),
                    content,
                    package,
                    cache,
                )
                .await?;
            }
        }

        Ok(())
    }

    fn find_package_source_directory(&self, package: &str) -> Result<PathBuf> {
        let package_path = package.replace('.', "/");

        if package.starts_with("java.") {
            if let Some(java_home) = &self.java_home {
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
                    let classes = self.find_classes_in_zip(&src_zip, package)?;
                    if !classes.is_empty() {
                        debug!(
                            "Found {} classes in src.zip for package {}",
                            classes.len(),
                            package
                        );
                        return Ok(src_zip);
                    }
                }
            }
        }

        if package.starts_with("groovy.") {
            if let Some(groovy_home) = &self.groovy_home {
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

        // FIXME: integrate with build tool to resolve imports
        if let Some(_) = &self.build_tool_home {}

        Err(anyhow!(
            "Could not find source directory for package: {}",
            package
        ))
    }

    fn find_classes_in_zip(&self, zip_path: &PathBuf, package: &str) -> Result<Vec<String>> {
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

    #[tracing::instrument(skip_all)]
    async fn parse_and_cache_builtin(
        &self,
        class_name: &str,
        source_path: PathBuf,
        zip_internal_path: Option<String>,
        content: String,
        package: &str,
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

        let builtin_info = BuiltinTypeInfo {
            source_path,
            zip_internal_path,
            tree,
            content,
            package: package.to_string(),
        };

        cache
            .builtin_infos
            .insert(class_name.to_string(), builtin_info);

        Ok(())
    }
}
