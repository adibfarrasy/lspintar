use std::path::Path;

use crate::vcs::handler::VcsHandler;

use anyhow::Result;

pub struct NoVcs;

impl VcsHandler for NoVcs {
    fn is_repository(&self, _root: &Path) -> bool {
        false
    }

    fn get_current_branch(&self) -> Result<String> {
        Ok("NONE".to_string())
    }
}
