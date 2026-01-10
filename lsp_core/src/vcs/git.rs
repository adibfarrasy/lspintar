use crate::vcs::handler::VcsHandler;
use anyhow::Result;
use std::path::Path;

pub struct GitHandler;

impl VcsHandler for GitHandler {
    fn is_repository(&self, root: &Path) -> bool {
        // TODO
        false
    }

    fn get_current_branch(&self) -> Result<String> {
        todo!()
    }
}
