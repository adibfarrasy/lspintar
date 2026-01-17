use std::path::Path;

use anyhow::Result;

use crate::vcs::VcsHandler;

pub struct NoVcs;

impl VcsHandler for NoVcs {
    fn is_repository(&self, _root: &Path) -> bool {
        false
    }

    fn get_current_branch(&self) -> Result<String> {
        Ok("NONE".to_string())
    }
}
