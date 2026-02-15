use std::sync::Arc;

use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};

mod as_lsp_location;
mod constants;
mod enums;
mod indexer;
mod models;
mod repo;
mod server;

use indexer::Indexer;
use repo::Repository;
use server::Backend;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_env_filter("debug,sqlx=warn,rusqlite=warn")
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .init();

    // TODO: put the sqlite file somewhere proper (user input with sane default)
    let db_dir = ":memory:";
    let db_dir = "/Users/adibf/Projects/lspintar-ws/lspintar/lspintar.db";
    let repo = Arc::new(Repository::new(db_dir).await.unwrap());

    let (service, socket) = LspService::new(|client| Backend::new(client, repo.clone()));

    Server::new(stdin(), stdout(), socket).serve(service).await;
}
