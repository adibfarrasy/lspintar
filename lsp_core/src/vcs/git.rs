use crate::vcs::handler::VcsHandler;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

pub struct GitHandler;

impl VcsHandler for GitHandler {
    fn is_repository(&self, root: &Path) -> bool {
        root.join(".git").exists()
    }

    fn get_current_branch(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            anyhow::bail!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        String::from_utf8(output.stdout)
            .context("Invalid UTF-8 in git output")
            .map(|s| s.trim().to_string())
    }
}
