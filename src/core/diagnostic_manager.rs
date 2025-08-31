use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Instant};
use tower_lsp::lsp_types::{Diagnostic, Url};
use tower_lsp::Client;

use crate::languages::LanguageRegistry;

#[derive(Debug, Clone)]
pub struct DiagnosticRequest {
    pub uri: String,
    pub content: String,
    pub version: i32,
    pub timestamp: Instant,
}

pub struct DiagnosticManager {
    pending_requests: Arc<RwLock<HashMap<String, DiagnosticRequest>>>,
    request_sender: mpsc::UnboundedSender<DiagnosticRequest>,
}

impl DiagnosticManager {
    pub fn new(client: Client, language_registry: Arc<LanguageRegistry>) -> Self {
        let (request_sender, request_receiver) = mpsc::unbounded_channel();
        let pending_requests = Arc::new(RwLock::new(HashMap::new()));

        let manager = Self {
            pending_requests: pending_requests.clone(),
            request_sender,
        };

        // Spawn background processor
        tokio::spawn(Self::process_requests(
            client,
            language_registry,
            pending_requests,
            request_receiver,
        ));

        manager
    }

    pub fn request_diagnostics(&self, uri: String, content: String, version: i32) {
        let request = DiagnosticRequest {
            uri: uri.clone(),
            content,
            version,
            timestamp: Instant::now(),
        };

        // Store pending request (replaces any existing one)
        if let Ok(mut pending) = self.pending_requests.try_write() {
            pending.insert(uri, request.clone());
        }

        let _ = self.request_sender.send(request);
    }

    async fn process_requests(
        client: Client,
        language_registry: Arc<LanguageRegistry>,
        pending_requests: Arc<RwLock<HashMap<String, DiagnosticRequest>>>,
        mut receiver: mpsc::UnboundedReceiver<DiagnosticRequest>,
    ) {
        const DEBOUNCE_MS: u64 = 300;

        while let Some(request) = receiver.recv().await {
            // Debouncing: wait a bit to see if more changes come
            sleep(Duration::from_millis(DEBOUNCE_MS)).await;

            let is_latest = {
                let pending = pending_requests.read().await;
                pending
                    .get(&request.uri)
                    .map(|latest| latest.timestamp >= request.timestamp)
                    .unwrap_or(false)
            };

            if !is_latest {
                continue;
            }

            if let Some(diagnostics) =
                Self::generate_diagnostics(&language_registry, &request).await
            {
                if let Ok(uri) = request.uri.parse::<Url>() {
                    client
                        .publish_diagnostics(uri, diagnostics, Some(request.version))
                        .await;
                }
            }

            {
                let mut pending = pending_requests.write().await;
                pending.remove(&request.uri);
            }
        }
    }

    async fn generate_diagnostics(
        language_registry: &LanguageRegistry,
        request: &DiagnosticRequest,
    ) -> Option<Vec<Diagnostic>> {
        let language_support = language_registry.detect_language(&request.uri)?;

        let content = request.content.clone();
        let language_support_clone = language_support.clone();

        tokio::task::spawn_blocking(move || {
            let mut parser = language_support_clone.create_parser();
            if let Some(tree) = parser.parse(&content, None) {
                Some(language_support_clone.collect_diagnostics(&tree, &content))
            } else {
                Some(vec![]) // Parser failed, but don't crash
            }
        })
        .await
        .ok()?
    }
}
