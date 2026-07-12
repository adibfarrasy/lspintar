pub mod gradle;
pub mod maven;
pub mod no_build_tool;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::build_tools::{gradle::GradleHandler, maven::MavenHandler, no_build_tool::NoBuildTool};

#[derive(Debug, Clone, PartialEq)]
pub enum BuildTool {
    Gradle,
    Maven,
}

/// Maps a single sub-project's source roots to the JARs on its compile/runtime classpath.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubprojectClasspath {
    pub source_dirs: Vec<PathBuf>,
    pub jar_paths: Vec<PathBuf>,
}

impl SubprojectClasspath {
    /// Returns true if `file` lives under one of this sub-project's source roots.
    pub fn contains_file(&self, file: &Path) -> bool {
        self.source_dirs.iter().any(|d| file.starts_with(d))
    }
}

pub fn get_build_tool(root: &Path) -> Arc<dyn BuildToolHandler + Send + Sync> {
    let providers: Vec<Arc<dyn BuildToolHandler>> =
        vec![Arc::new(GradleHandler), Arc::new(MavenHandler)];
    providers
        .into_iter()
        .find(|p| p.is_project(root))
        .unwrap_or_else(|| Arc::new(NoBuildTool))
}

/// Splits jar paths into bytecode/source jars and pairs them by base name,
/// e.g. `guava-33.0.jar` <-> `guava-33.0-sources.jar`.
pub fn pair_jars_with_sources(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<(Option<PathBuf>, Option<PathBuf>)> {
    let (source_jars, bytecode_jars): (Vec<PathBuf>, Vec<PathBuf>) = paths
        .into_iter()
        .filter(|p| p.exists())
        .partition(|p| p.to_string_lossy().ends_with("-sources.jar"));

    let base_name = |path: &Path, strip: bool| -> Option<String> {
        path.file_stem().and_then(|s| s.to_str()).map(|name| {
            if strip {
                name.trim_end_matches("-sources").to_string()
            } else {
                name.to_string()
            }
        })
    };

    let source_map: HashMap<String, PathBuf> = source_jars
        .into_iter()
        .filter_map(|path| base_name(&path, true).map(|n| (n, path)))
        .collect();

    let bytecode_map: HashMap<String, PathBuf> = bytecode_jars
        .into_iter()
        .filter_map(|path| base_name(&path, false).map(|n| (n, path)))
        .collect();

    let mut pairs: Vec<(Option<PathBuf>, Option<PathBuf>)> = source_map
        .iter()
        .map(|(name, src)| (bytecode_map.get(name).cloned(), Some(src.clone())))
        .collect();

    // bytecode-only jars (no source)
    for (name, byte) in &bytecode_map {
        if !source_map.contains_key(name) {
            pairs.push((Some(byte.clone()), None));
        }
    }

    pairs
}

pub trait BuildToolHandler: Send + Sync {
    fn is_project(&self, root: &Path) -> bool;
    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(Option<PathBuf>, Option<PathBuf>)>>;
    fn get_jdk_dependency_path(&self, root: &Path) -> Result<Option<PathBuf>>;
    fn is_build_file(&self, path: &Path) -> bool;
    /// Returns the per-sub-project source-root → classpath JAR mapping.
    /// Returns an empty vec for single-project setups or when not applicable.
    fn get_subproject_classpath(&self, root: &Path) -> Result<Vec<SubprojectClasspath>>;
}
