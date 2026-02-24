use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::vcs::{git::GitHandler, no_vcs::NoVcs};

use anyhow::Result;

pub mod git;
pub mod no_vcs;

pub fn get_vcs_handler(root: &Path) -> Arc<dyn VcsHandler> {
    let providers: Vec<Arc<dyn VcsHandler>> = vec![Arc::new(GitHandler)];
    providers
        .into_iter()
        .find(|p| p.is_repository(root))
        .unwrap_or_else(|| Arc::new(NoVcs))
}

pub trait VcsHandler: Send + Sync {
    fn is_repository(&self, root: &Path) -> bool;
    fn get_current_revision(&self) -> Result<String>;
    fn get_changed_files(&self, old_rev: &str, new_rev: &str, root: &Path) -> Result<Vec<PathBuf>>;
    fn get_revision_file(&self, root: &Path) -> Option<PathBuf>;
}
