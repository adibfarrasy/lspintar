use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::vcs::VcsHandler;

pub struct GitHandler;

impl VcsHandler for GitHandler {
    fn is_repository(&self, root: &Path) -> bool {
        root.join(".git").exists()
    }

    fn get_current_revision(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
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

    fn get_changed_files(&self, old_rev: &str, new_rev: &str, root: &Path) -> Result<Vec<PathBuf>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", old_rev, new_rev])
            .current_dir(root)
            .output()
            .context("Failed to execute git command")?;
        if !output.status.success() {
            anyhow::bail!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let out = String::from_utf8(output.stdout).context("Invalid UTF-8 in git output")?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| root.join(l))
            .collect())
    }

    fn get_revision_file(&self, root: &Path) -> Option<PathBuf> {
        Some(root.join(".git/HEAD"))
    }
}
