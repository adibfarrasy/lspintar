use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::build_tools::{BuildToolHandler, SubprojectClasspath};

pub struct NoBuildTool;

impl BuildToolHandler for NoBuildTool {
    fn is_project(&self, _root: &Path) -> bool {
        true
    }

    fn get_dependency_paths(
        &self,
        _root: &Path,
    ) -> Result<Vec<(Option<PathBuf>, Option<PathBuf>)>> {
        Ok(vec![])
    }

    fn get_jdk_dependency_path(&self, _root: &Path) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    fn is_build_file(&self, _path: &Path) -> bool {
        false
    }

    fn get_subproject_classpath(&self, _root: &Path) -> Result<Vec<SubprojectClasspath>> {
        Ok(vec![])
    }
}
