pub mod gradle;
pub mod no_build_tool;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;

use crate::build_tools::{gradle::GradleHandler, no_build_tool::NoBuildTool};

#[derive(Debug, Clone, PartialEq)]
pub enum BuildTool {
    Gradle,
    Maven,
}

pub fn get_build_tool(root: &Path) -> Arc<dyn BuildToolHandler> {
    let providers: Vec<Arc<dyn BuildToolHandler>> = vec![Arc::new(GradleHandler)];
    providers
        .into_iter()
        .find(|p| p.is_project(root))
        .unwrap_or_else(|| Arc::new(NoBuildTool))
}

pub trait BuildToolHandler: Send + Sync {
    fn is_project(&self, root: &Path) -> bool;
    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(PathBuf, Option<PathBuf>)>>;
}
