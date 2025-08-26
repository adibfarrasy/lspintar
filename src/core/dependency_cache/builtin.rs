use anyhow::{anyhow, Context, Result};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tracing::debug;
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::core::build_tools::ExternalDependency;
use crate::languages::groovy::constants::GROOVY_DEFAULT_IMPORTS;
use crate::languages::java::constants::JAVA_COMMON_IMPORTS;

use super::source_file_info::SourceFileInfo;
use super::DependencyCache;

pub struct BuiltinResolver {
    dependency_paths: Vec<PathBuf>,
}

impl BuiltinResolver {
    pub fn new() -> Self {
        let mut dependency_paths = Vec::new();

        let java_home = std::env::var("JAVA_HOME").ok().map(PathBuf::from);
        let groovy_home = std::env::var("GROOVY_HOME").ok().map(PathBuf::from);

        let java_import_paths: Vec<PathBuf> = JAVA_COMMON_IMPORTS
            .iter()
            .map(|i| find_package_source_directory(i, &java_home, &None))
            .filter_map(|result| result.ok())
            .collect();

        dependency_paths.extend_from_slice(&java_import_paths);

        let groovy_import_paths: Vec<PathBuf> = GROOVY_DEFAULT_IMPORTS
            .iter()
            .map(|i| find_package_source_directory(i, &java_home, &groovy_home))
            .filter_map(|result| result.ok())
            .collect();

        dependency_paths.extend_from_slice(&groovy_import_paths);

        Self { dependency_paths }
    }

    #[tracing::instrument(skip_all)]
    pub async fn index_builtin_dependencies(&self, cache: Arc<DependencyCache>) -> Result<()> {
        let futures: Vec<_> = self
            .dependency_paths
            .iter()
            .map(|import_path| self.load_package_classes(import_path, cache.clone()))
            .collect();

        futures::future::try_join_all(futures).await?;

        debug!("external dependencies initialized.");

        Ok(())
    }

    #[tracing::instrument(skip(self, cache))]
    async fn load_package_classes(
        &self,
        source_path: &PathBuf,
        cache: Arc<DependencyCache>,
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
                self.load_from_zip(&entry.path().to_path_buf(), cache.clone())
                    .await?;
            } else {
                let file_path = entry.path();

                if let Some(class_name) = file_path.file_stem().and_then(|s| s.to_str()) {
                    // For regular files, try to derive package from path structure
                    // This is a best-effort approach
                    let qualified_name = if let Some(source_path_str) = source_path.to_str() {
                        if let Some(file_path_str) = file_path.to_str() {
                            if let Some(relative_path) = file_path_str.strip_prefix(source_path_str)
                            {
                                relative_path
                                    .trim_start_matches('/')
                                    .trim_end_matches(".java")
                                    .trim_end_matches(".groovy")
                                    .replace('/', ".")
                            } else {
                                class_name.to_string()
                            }
                        } else {
                            class_name.to_string()
                        }
                    } else {
                        class_name.to_string()
                    };

                    parse_and_cache_builtin(
                        &qualified_name,
                        file_path.to_path_buf(),
                        None,
                        None,
                        &cache.clone(),
                    )
                    .with_context(|| {
                        format!("Failed to parse external classes: {}", class_name)
                    })?;
                }
            }
        }

        Ok(())
    }

    async fn load_from_zip(&self, zip_path: &PathBuf, cache: Arc<DependencyCache>) -> Result<()> {
        let zip_data = tokio::fs::read(zip_path).await?;

        tokio::task::spawn_blocking({
            let zip_path = zip_path.clone();
            move || -> Result<()> {
                let cursor = Cursor::new(zip_data);
                let mut archive = ZipArchive::new(cursor)?;

                let entries: Vec<String> = (0..archive.len())
                    .filter_map(|i| {
                        let file = archive.by_index(i).ok()?;
                        let file_name = file.name().to_string();

                        if !(file_name.ends_with(".java") || file_name.ends_with(".groovy")) {
                            return None;
                        }

                        if !should_index_package(&file_name) {
                            return None;
                        }

                        Some(file_name)
                    })
                    .collect();

                let chunk_size = std::cmp::max(1, entries.len() / num_cpus::get());
                let mut handles = Vec::new();

                for chunk in entries.chunks(chunk_size) {
                    let chunk = chunk.to_vec();
                    let zip_path = zip_path.clone();
                    let cache = cache.clone();

                    let handle = thread::spawn(move || -> Result<()> {
                        for file_name in chunk {
                            // Convert file path to fully qualified name
                            let mut package_path = file_name
                                .trim_end_matches(".java")
                                .trim_end_matches(".groovy")
                                .replace('/', ".");
                            
                            // Handle Java 9+ modular format: remove module prefix (java.base.java.lang.String -> java.lang.String)
                            if package_path.starts_with("java.base.java.") {
                                package_path = package_path.strip_prefix("java.base.").unwrap().to_string();
                            } else if package_path.starts_with("java.desktop.java.") {
                                package_path = package_path.strip_prefix("java.desktop.").unwrap().to_string();
                            } else if package_path.contains(".java.") {
                                // Generic handling for other modules: module_name.java.lang.String -> java.lang.String
                                if let Some(java_pos) = package_path.find(".java.") {
                                    package_path = package_path[java_pos + 1..].to_string();
                                }
                            }

                            parse_and_cache_builtin(
                                &package_path,
                                zip_path.clone(),
                                Some(file_name.clone()),
                                None,
                                &cache,
                            )?;
                        }

                        Ok(())
                    });

                    handles.push(handle);
                }

                for handle in handles {
                    handle
                        .join()
                        .map_err(|_| anyhow::anyhow!("Thread panicked"))??;
                }
                Ok(())
            }
        })
        .await??;

        Ok(())
    }
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

            // Try both old (Java 8) and new (Java 9+) src.zip locations
            let src_zip_candidates = [
                java_home.join("src.zip"),      // Java 8 and earlier
                java_home.join("lib/src.zip"),  // Java 9 and later
            ];
            
            for src_zip in &src_zip_candidates {
                if src_zip.exists() {
                    let classes = find_classes_in_zip(src_zip, &package_path)?;

                    if !classes.is_empty() {
                        return Ok(src_zip.clone());
                    }
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
    // Also check for modular format (Java 9+): module_name/package/path/
    let modular_package_prefix = format!("/{}", package_prefix);

    let classes: Vec<String> = archive
        .file_names()
        .filter(|name| {
            // Handle both old format (java/lang/) and new modular format (java.base/java/lang/)
            let matches_old_format = name.starts_with(&package_prefix)
                && (name.ends_with(".java") || name.ends_with(".groovy"))
                && name.matches('/').count() == package_prefix.matches('/').count();
                
            let matches_modular_format = name.contains(&modular_package_prefix)
                && (name.ends_with(".java") || name.ends_with(".groovy"))
                && name.split('/').last().map(|f| !f.is_empty()).unwrap_or(false);
                
            matches_old_format || matches_modular_format
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
fn parse_and_cache_builtin(
    class_name: &str,
    source_path: PathBuf,
    zip_internal_path: Option<String>,
    dependency: Option<ExternalDependency>,
    cache: &DependencyCache,
) -> Result<()> {
    let external_info = SourceFileInfo::new(source_path, zip_internal_path, dependency);

    cache
        .builtin_infos
        .insert(class_name.to_string(), external_info);

    Ok(())
}

fn should_index_package(file_path: &str) -> bool {
    const HIGH_PRIORITY: &[&str] = &[
        "java/lang/",
        "java/util/",
        "java/io/",
        "java/math/",
        "java/net/",
        "java/text/",
        "java/security/",
        "groovy/lang/",
        "groovy/util/",
    ];

    const SKIP_PACKAGES: &[&str] = &[
        "jdk/",
        "sun/",
        "com/sun/",
        "java/awt/",
        "javax/swing/",
        "java/applet/",
        "javax/imageio/",
        "javax/print/",
        "javax/sound/",
    ];

    if SKIP_PACKAGES.iter().any(|skip| file_path.starts_with(skip)) {
        return false;
    }

    if HIGH_PRIORITY.iter().any(|pkg| file_path.starts_with(pkg)) {
        return true;
    }

    // NOTE: include everything else that's not explicitly skipped
    true
}
