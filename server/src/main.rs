use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};

mod constants;
mod enums;
mod indexer;
mod lsp_convert;
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

    let (service, socket) = LspService::new(|client| Backend::new(client));

    Server::new(stdin(), stdout(), socket).serve(service).await;
}
