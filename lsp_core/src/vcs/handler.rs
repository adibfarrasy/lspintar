use anyhow::Result;
use std::path::Path;

pub trait VcsHandler: Send + Sync {
    fn is_repository(&self, root: &Path) -> bool;
    fn get_current_branch(&self) -> Result<String>;
}
