use std::{
    env,
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

pub struct TestServer {
    pub backend: Backend,
    _temp_file: tempfile::NamedTempFile,
}

impl TestServer {
    async fn new(fixture: &str) -> Self {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let db_dir = format!("sqlite:{}", temp_file.path().display());
        let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
        let (service, _socket) = LspService::new(|client| Backend::new(client));
        let backend = service.inner().clone();
        backend.repo.set(repo).ok();
        let root = env::current_dir().expect("cannot get current dir");

        let mut init_params = InitializeParams::default();
        init_params.root_uri = Some(
            Url::from_file_path(root.join("tests/fixtures").join(fixture))
                .expect("cannot parse root URI"),
        );

        backend.initialize(init_params).await.unwrap();
        backend.initialized(InitializedParams {}).await;
        Self {
            backend,
            _temp_file: temp_file,
        }
    }
}

static TEST_SERVERS: LazyLock<DashMap<&'static str, Arc<OnceCell<Arc<TestServer>>>>> =
    LazyLock::new(DashMap::new);

pub async fn get_test_server(fixture: &'static str) -> Arc<TestServer> {
    let cell = TEST_SERVERS
        .entry(fixture)
        .or_insert_with(|| Arc::new(OnceCell::new()))
        .clone();

    let server = cell
        .get_or_init(|| async { Arc::new(TestServer::new(fixture).await) })
        .await
        .clone();

    server
}
