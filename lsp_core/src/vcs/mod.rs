use std::path::Path;

use crate::vcs::{git::GitHandler, handler::VcsHandler, no_vcs::NoVcs};

pub mod git;
pub mod handler;
pub mod no_vcs;

pub fn get_vcs_handler(root: &Path) -> Box<dyn VcsHandler> {
    let providers: Vec<Box<dyn VcsHandler>> = vec![Box::new(GitHandler)];
    providers
        .into_iter()
        .find(|p| p.is_repository(root))
        .unwrap_or_else(|| Box::new(NoVcs))
}
