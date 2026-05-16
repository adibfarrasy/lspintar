use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use tower_lsp::LanguageServer;

use lspintar_server::{Repository, server::Backend};
use tower_lsp::{
    LspService,
    lsp_types::{InitializeParams, InitializedParams, Url},
};

use dashmap::DashMap;
use tokio::sync::OnceCell;

// Each integration-test binary uses only a subset of this helper's API, so
// missing-use warnings per binary are expected.
#[allow(dead_code)]
pub struct TestServer {
    pub backend: Backend,
    root: PathBuf,
    _temp_dir: tempfile::TempDir,
}

#[allow(dead_code)]
impl TestServer {
    /// Workspace root for this test server. All paths used by tests must be
    /// derived from this — never from `env::current_dir()` — so they point
    /// inside the per-binary copied fixture instead of the on-disk original.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Build a `file://` URI for a path inside the workspace.
    pub fn uri(&self, relative: &str) -> Url {
        Url::from_file_path(self.root.join(relative)).expect("cannot build URI")
    }

    async fn new(fixture: &str) -> Self {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let src_root = std::env::current_dir()
            .expect("cannot get current dir")
            .join("tests/fixtures")
            .join(fixture);
        let dest_root = temp_dir.path().join(fixture);
        copy_fixture(&src_root, &dest_root).expect("copy fixture");

        let db_path = temp_dir.path().join("test.db");
        let db_url = format!("sqlite:{}", db_path.display());
        let repo = Arc::new(Repository::new(&db_url).await.unwrap());
        let (service, _socket) = LspService::new(|client| Backend::new(client));
        let backend = service.inner().clone();
        backend.repo.set(repo).ok();

        let mut init_params = InitializeParams::default();
        init_params.root_uri =
            Some(Url::from_file_path(&dest_root).expect("cannot parse root URI"));

        backend.initialize(init_params).await.unwrap();
        backend.initialized(InitializedParams {}).await;
        Self {
            backend,
            root: dest_root,
            _temp_dir: temp_dir,
        }
    }
}

/// Recursive copy with a small skip-list. The build output dirs are dropped
/// to keep the copy cheap; Gradle caches and any prior `.lspintar` artefacts
/// would just leak global state into the per-binary fixture, so they are
/// excluded too.
fn copy_fixture(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some("build" | ".gradle" | ".lspintar" | "target" | "node_modules" | ".git")
        ) {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_fixture(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

static TEST_SERVERS: LazyLock<DashMap<&'static str, Arc<OnceCell<Arc<TestServer>>>>> =
    LazyLock::new(DashMap::new);

pub async fn get_test_server(fixture: &'static str) -> Arc<TestServer> {
    let cell = TEST_SERVERS
        .entry(fixture)
        .or_insert_with(|| Arc::new(OnceCell::new()))
        .clone();

    cell.get_or_init(|| async { Arc::new(TestServer::new(fixture).await) })
        .await
        .clone()
}
