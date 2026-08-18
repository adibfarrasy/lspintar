// Tests for content-hash-driven incremental reindexing.
//
// Change detection is content-addressed (sha256 per file) rather than tied to
// a VCS revision pointer, so switching branches back and forth reuses the
// index instead of re-diffing against whatever revision was last recorded.

use lspintar_server::constants::FILE_HASHES_PATH_FRAGMENT;

/// The FILE_HASHES_PATH_FRAGMENT constant must sit inside the .lspintar
/// directory alongside the other persisted index artefacts.
#[test]
fn file_hashes_fragment_is_under_lspintar_dir() {
    assert!(
        FILE_HASHES_PATH_FRAGMENT.starts_with(".lspintar/"),
        "FILE_HASHES_PATH_FRAGMENT should be under .lspintar/, got: {FILE_HASHES_PATH_FRAGMENT}"
    );
}
