use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Content hash of a file, used to detect real changes independent of mtime
/// or VCS revision — a branch switch that leaves a file's bytes untouched
/// produces the same hash, so callers can skip re-indexing it.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_changes_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let h1 = hash_file(&path).unwrap();
        std::fs::write(&path, b"world").unwrap();
        let h2 = hash_file(&path).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_stable_for_same_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(hash_file(&path).unwrap(), hash_file(&path).unwrap());
    }

    #[test]
    fn missing_file_errors() {
        let path = Path::new("/nonexistent/does-not-exist.txt");
        assert!(hash_file(path).is_err());
    }
}
