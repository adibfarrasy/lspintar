use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::build_tools::BuildToolHandler;

pub struct NoBuildTool;

impl BuildToolHandler for NoBuildTool {
    fn is_project(&self, _root: &Path) -> bool {
        true
    }

    fn get_dependency_paths(&self, _root: &Path) -> Result<Vec<(PathBuf, Option<PathBuf>)>> {
        Ok(vec![])
    }
}
