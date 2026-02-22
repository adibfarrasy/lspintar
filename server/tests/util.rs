use std::{
    env,
    sync::{Arc, LazyLock},
};
use tower_lsp::{ClientSocket, LanguageServer};

use lspintar_server::{Repository, server::Backend};
use tower_lsp::{
    LspService,
    lsp_types::{InitializeParams, InitializedParams, Url},
};
use uuid::Uuid;

use dashmap::DashMap;
use tokio::sync::OnceCell;

pub struct TestServer {
    pub backend: Backend,
    _socket: ClientSocket,
}

impl TestServer {
    async fn new(fixture: &str) -> Self {
        let db_name = Uuid::new_v4();
        let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
        let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
        let (service, socket) = LspService::new(|client| Backend::new(client, repo.clone()));
        let backend = service.inner().clone();
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
            _socket: socket,
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

    cell.get_or_init(|| async { Arc::new(TestServer::new(fixture).await) })
        .await
        .clone()
}
