use std::sync::Arc;

use crate::languages::{groovy::GroovySupport, LanguageRegistry};
use server::LspServer;
use tokio::io::{stdin, stdout};
use tower_lsp::{LspService, Server};

mod constants;
mod core;
mod languages;
mod server;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
        .target(env_logger::Target::Stderr)
        .init();

    let mut registry = LanguageRegistry::new();
    registry.register("groovy", Box::new(GroovySupport::new()));

    // Future
    // registry.register("kotlin", Box::new(KotlinSupport::new()));
    // registry.register("java", Box::new(JavaSupport::new()));

    let (service, socket) = LspService::new(|client| LspServer::new(client, Arc::new(registry)));

    Server::new(stdin(), stdout(), socket).serve(service).await;
}
