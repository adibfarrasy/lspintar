// Tests for VCS-driven incremental reindexing (lspintar.allium open question 1).
//
// When the workspace has no VCS (NoVcs), no revision file is written.
// When the workspace has a git repo and the revision changes between startups,
// IncrementalOpen queues only the changed source files for re-indexing instead
// of re-indexing everything.
//
// The integration-test feature forces needs_full_reindex = true, so the
// IncrementalOpen path itself is tested via unit tests in lsp_core::vcs::git.
// The tests here cover the observable side-effects of the full-reindex path.

use std::env;

use crate::util::get_test_server;
use lspintar_server::constants::VCS_REVISION_PATH_FRAGMENT;

mod util;

/// After initializing against a fixture that has no .git directory, the VCS
/// revision file must NOT be written — there is nothing to track.
#[tokio::test]
async fn no_vcs_workspace_leaves_no_revision_file() {
    let server = get_test_server("polyglot-spring").await;
    let root = env::current_dir()
        .expect("cannot get current dir")
        .join("tests/fixtures/polyglot-spring");

    let _ = server; // ensure initialization has run

    let revision_path = root.join(VCS_REVISION_PATH_FRAGMENT);
    assert!(
        !revision_path.exists(),
        "revision file should not exist for a NoVcs workspace, found: {}",
        revision_path.display()
    );
}

/// The VCS_REVISION_PATH_FRAGMENT constant must sit inside the .lspintar
/// directory alongside the other persisted index artefacts.
#[test]
fn vcs_revision_fragment_is_under_lspintar_dir() {
    assert!(
        VCS_REVISION_PATH_FRAGMENT.starts_with(".lspintar/"),
        "VCS_REVISION_PATH_FRAGMENT should be under .lspintar/, got: {VCS_REVISION_PATH_FRAGMENT}"
    );
}
