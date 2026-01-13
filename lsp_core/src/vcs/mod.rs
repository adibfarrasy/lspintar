use std::{path::Path, sync::Arc};

use crate::vcs::{git::GitHandler, handler::VcsHandler, no_vcs::NoVcs};

pub mod git;
pub mod handler;
pub mod no_vcs;

pub fn get_vcs_handler(root: &Path) -> Arc<dyn VcsHandler> {
    let providers: Vec<Arc<dyn VcsHandler>> = vec![Arc::new(GitHandler)];
    providers
        .into_iter()
        .find(|p| p.is_repository(root))
        .unwrap_or_else(|| Arc::new(NoVcs))
}
