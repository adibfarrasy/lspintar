pub mod gradle;
pub mod no_build_tool;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::build_tools::{gradle::GradleHandler, no_build_tool::NoBuildTool};

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
    let providers: Vec<Arc<dyn BuildToolHandler>> = vec![Arc::new(GradleHandler)];
    providers
        .into_iter()
        .find(|p| p.is_project(root))
        .unwrap_or_else(|| Arc::new(NoBuildTool))
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
