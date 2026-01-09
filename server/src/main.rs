use server::LspServer;
use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};

mod indexer;
mod models;
mod repo;
mod server;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .init();

    let (service, socket) = LspService::new(|client| LspServer::new(client));

    Server::new(stdin(), stdout(), socket).serve(service).await;
}
