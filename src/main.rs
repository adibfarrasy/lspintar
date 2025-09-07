use std::sync::Arc;

use crate::languages::{groovy::GroovySupport, java::JavaSupport, kotlin::KotlinSupport, LanguageRegistry};
use server::LspServer;
use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};

mod constants;
mod core;
mod languages;
mod server;
mod types;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .init();

    let mut registry = LanguageRegistry::new();
    registry.register("groovy", Box::new(GroovySupport::new()));
    registry.register("java", Box::new(JavaSupport::new()));

    registry.register("kotlin", Box::new(KotlinSupport::new()));

    let (service, socket) = LspService::new(|client| LspServer::new(client, Arc::new(registry)));

    Server::new(stdin(), stdout(), socket).serve(service).await;
}
