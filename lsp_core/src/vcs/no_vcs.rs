use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::vcs::VcsHandler;

pub struct NoVcs;

impl VcsHandler for NoVcs {
    fn is_repository(&self, _root: &Path) -> bool {
        false
    }

    fn get_current_revision(&self) -> Result<String> {
        anyhow::bail!("No VCS available")
    }

    fn get_changed_files(
        &self,
        _old_rev: &str,
        _new_rev: &str,
        _root: &Path,
    ) -> Result<Vec<PathBuf>> {
        anyhow::bail!("No VCS available")
    }

    fn get_revision_file(&self, _root: &Path) -> Option<PathBuf> {
        None
    }
}
