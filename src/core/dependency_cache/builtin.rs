use std::path::PathBuf;

use anyhow::{Context, Result};
use tree_sitter::Tree;

use crate::core::{constants::GROOVY_DEFAULT_IMPORTS, utils::create_parser_for_language};

use super::DependencyCache;

#[derive(Debug, Clone)]
pub struct BuiltinTypeInfo {
    pub source_path: PathBuf,
    pub tree: Tree,
    pub content: String,
    pub package: String,
}

pub struct BuiltinResolver {
    java_home: Option<PathBuf>,
    groovy_home: Option<PathBuf>,
    gradle_home: Option<PathBuf>,
}

impl BuiltinResolver {
    pub fn new() -> Self {
        Self {
            java_home: std::env::var("JAVA_HOME").ok().map(PathBuf::from),
            groovy_home: std::env::var("GROOVY_HOME").ok().map(PathBuf::from),
            gradle_home: std::env::var("GRADLE_HOME").ok().map(PathBuf::from),
        }
    }

    pub async fn initialize_builtins(&self, cache: &DependencyCache) -> Result<()> {
        for import_pattern in GROOVY_DEFAULT_IMPORTS {
            self.resolve_import_pattern(import_pattern, cache).await?;
        }

        Ok(())
    }

    async fn resolve_import_pattern(
        &self,
        import_pattern: &str,
        cache: &DependencyCache,
    ) -> Result<()> {
        if import_pattern.ends_with(".*") {
            let package = &import_pattern[..import_pattern.len() - 2]; // Remove ".*"
            self.load_package_classes(package, cache).await?;
        } else {
            // Handle specific class imports (e.g., "java.math.BigDecimal")
            self.load_specific_class(import_pattern, cache).await?;
        }
        Ok(())
    }

    async fn load_package_classes(&self, package: &str, cache: &DependencyCache) -> Result<()> {
        // Step 1: Find source directory for package
        let source_dir = self
            .find_package_source_directory(package)
            .context(format!(
                "Failed to find source directory for package: {}",
                package
            ))?;

        // Step 2: Scan directory for .java/.groovy files
        let source_files = std::fs::read_dir(&source_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext == "java" || ext == "groovy")
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        // Step 3: Parse each source file
        for entry in source_files {
            let file_path = entry.path();
            if let Some(class_name) = file_path.file_stem().and_then(|s| s.to_str()) {
                self.parse_and_cache_builtin(class_name, &file_path, package, cache)
                    .await
                    .context(format!("Failed to parse builtin class: {}", class_name))?;
            }
        }

        // Step 4: Cache the package directory mapping
        cache
            .builtin_packages
            .insert(format!("{}.*", package), source_dir);

        Ok(())
    }

    async fn load_specific_class(
        &self,
        full_class_name: &str,
        cache: &DependencyCache,
    ) -> Result<()> {
        // Step 1: Split package and class name
        let parts: Vec<&str> = full_class_name.rsplitn(2, '.').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid class name: {}", full_class_name));
        }

        let class_name = parts[0];
        let package = parts[1];

        // Step 2: Find source file
        let source_file = self
            .find_class_source_file(package, class_name)
            .context(format!(
                "Failed to find source file for: {}",
                full_class_name
            ))?;

        // Step 3: Parse and cache
        self.parse_and_cache_builtin(class_name, &source_file, package, cache)
            .await?;

        Ok(())
    }

    fn find_package_source_directory(&self, package: &str) -> Result<PathBuf> {
        let package_path = package.replace('.', "/");

        // Step 1: Try JAVA_HOME for java.* packages
        if package.starts_with("java.") {
            if let Some(java_home) = &self.java_home {
                let candidates = [
                    java_home.join("src").join(&package_path), // OpenJDK layout
                    java_home.join("lib").join("src").join(&package_path), // Some distributions
                    java_home.join("src.zip").join(&package_path), // If extracted
                ];

                for candidate in &candidates {
                    if candidate.exists() {
                        return Ok(candidate.clone());
                    }
                }
            }
        }

        // Step 2: Try GROOVY_HOME for groovy.* packages
        if package.starts_with("groovy.") {
            if let Some(groovy_home) = &self.groovy_home {
                let candidates = [
                    groovy_home
                        .join("src")
                        .join("main")
                        .join("java")
                        .join(&package_path),
                    groovy_home
                        .join("src")
                        .join("main")
                        .join("groovy")
                        .join(&package_path),
                    groovy_home.join("src").join(&package_path),
                ];

                for candidate in &candidates {
                    if candidate.exists() {
                        return Ok(candidate.clone());
                    }
                }
            }
        }

        // Step 3: Try Gradle cache for any package
        if let Some(gradle_home) = &self.gradle_home {
            let cache_dir = gradle_home
                .join("caches")
                .join("modules-2")
                .join("files-2.1");
            // This would require more complex logic to find the right JAR and extract source
            // For now, skip this implementation
            // TODO: fix this
        }

        Err(anyhow::anyhow!(
            "Could not find source directory for package: {}",
            package
        ))
    }

    fn find_class_source_file(&self, package: &str, class_name: &str) -> Result<PathBuf> {
        let package_dir = self.find_package_source_directory(package)?;

        let candidates = [
            package_dir.join(format!("{}.java", class_name)),
            package_dir.join(format!("{}.groovy", class_name)),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }

        Err(anyhow::anyhow!(
            "Could not find source file for class: {}.{}",
            package,
            class_name
        ))
    }

    async fn parse_and_cache_builtin(
        &self,
        class_name: &str,
        source_path: &PathBuf,
        package: &str,
        cache: &DependencyCache,
    ) -> Result<()> {
        // Step 1: Read source file
        let content = tokio::fs::read_to_string(source_path)
            .await
            .context("Failed to read source file")?;

        // Step 2: Create appropriate parser
        let language = if source_path.extension().and_then(|s| s.to_str()) == Some("groovy") {
            "groovy"
        } else {
            "java"
        };

        let mut parser = create_parser_for_language(language).context("Failed to create parser")?;

        // Step 3: Parse source
        let tree = parser
            .parse(&content, None)
            .context("Failed to parse source file")?;

        // Step 4: Cache the parsed result
        let builtin_info = BuiltinTypeInfo {
            source_path: source_path.clone(),
            tree,
            content,
            package: package.to_string(),
        };

        cache
            .builtin_trees
            .insert(class_name.to_string(), builtin_info);

        Ok(())
    }
}
