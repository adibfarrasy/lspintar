use anyhow::anyhow;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;

use crate::core::dependency_cache::BuiltinResolver;
use crate::core::dependency_cache::DependencyCache;
use crate::core::DiagnosticManager;
use crate::core::Document;
use crate::core::DocumentManager;
use crate::languages::{LanguageRegistry, LanguageSupport};

pub struct LspServer {
    documents: Arc<RwLock<DocumentManager>>,
    language_registry: Arc<LanguageRegistry>,
    diagnostics: Arc<DashMap<String, DiagnosticManager>>,
    dependency_cache: Arc<DependencyCache>,
    client: tower_lsp::Client,
}

#[tower_lsp::async_trait]
impl LanguageServer for LspServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "LSP server initialized")
            .await;

        // let cache = self.dependency_cache.clone();
        // let client = self.client.clone();

        // tokio::spawn(async move {
        //     let resolver = BuiltinResolver::new();
        //
        //     match resolver.initialize_builtins(&cache).await {
        //         Ok(_) => {
        //             client
        //                 .log_message(
        //                     MessageType::INFO,
        //                     "Builtin types cache initialized successfully",
        //                 )
        //                 .await;
        //         }
        //         Err(e) => {
        //             client
        //                 .log_message(
        //                     MessageType::WARNING,
        //                     &format!("Failed to initialize builtin cache: {}", e),
        //                 )
        //                 .await;
        //         }
        //     }
        // });
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let text_document = params.text_document;
        let uri = text_document.uri.to_string();

        // Detect language (verify it's supported)
        let language_support = self.language_registry.detect_language(&uri);
        if language_support.is_none() {
            return;
        }

        // Store document content
        let document = Document::new(
            uri.clone(),
            text_document.text.clone(),
            text_document.version,
            text_document.language_id.clone(),
        );

        {
            let mut documents = self.documents.write().await;
            documents.insert(document);

            documents.reparse_and_cache_tree(&uri, &text_document.text, &self.language_registry);
        }

        // Trigger initial diagnostics
        self.diagnostics
            .entry(uri.clone())
            .or_insert_with(|| {
                DiagnosticManager::new(self.client.clone(), self.language_registry.clone())
            })
            .request_diagnostics(uri, text_document.text, text_document.version);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        // Update document content
        let (content, version) = {
            let mut documents = self.documents.write().await;
            if let Some(doc) = documents.update_content(
                params.text_document,
                params.content_changes,
                &self.language_registry,
            ) {
                (doc.content.clone(), doc.version)
            } else {
                return; // No document found
            }
        };

        // Debounced diagnostic refresh
        self.diagnostics
            .entry(uri.clone())
            .or_insert_with(|| {
                DiagnosticManager::new(self.client.clone(), self.language_registry.clone())
            })
            .request_diagnostics(uri, content, version);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        // Remove from document manager
        {
            let mut document = self.documents.write().await;
            document.remove(&uri);
        }

        // Clear diagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;

        // Clean up language-specific state
        self.diagnostics.remove(&uri);
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();

        let position = params.text_document_position_params.position;

        let language_support = self
            .language_registry
            .detect_language(&uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

        let (content, tree) = {
            let document_manager = self.documents.read().await;
            let document =
                document_manager
                    .get(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "cannot find document with uri {}",
                        uri
                    )))?;

            let tree = document_manager
                .get_tree(&uri)
                .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

            // Clone the data you need
            (document.content.clone(), tree.clone())
        };

        let cache = self.dependency_cache.clone();

        let result = tokio::task::spawn_blocking(move || {
            language_support.find_definition(&tree, &content, position, &uri, cache)
        })
        .await
        .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())?;

        match result {
            Ok(location) => Ok(Some(GotoDefinitionResponse::Scalar(location))),
            Err(error) => Err(tower_lsp::jsonrpc::Error::invalid_params(error.to_string())),
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        // Language-specific hover info
        todo!()
    }

    // Future features ===
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        // Language-specific completion
        todo!()
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        // Language-specific code actions
        todo!()
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        // Language-specific formatting
        todo!()
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        // Language-specific reference finding
        todo!()
    }
}

impl LspServer {
    pub fn new(client: tower_lsp::Client, registry: Arc<LanguageRegistry>) -> Self {
        Self {
            documents: Arc::new(RwLock::new(DocumentManager::new())),
            language_registry: registry,
            diagnostics: Arc::new(DashMap::new()),
            client,
            dependency_cache: Arc::new(DependencyCache::new()),
        }
    }

    // Helper: Get language support for document
    async fn get_language_support(&self, uri: &str) -> Option<Arc<dyn LanguageSupport>> {
        self.language_registry.detect_language(uri)
    }

    // Helper: Publish diagnostics
    async fn publish_diagnostics(&self, uri: String, diagnostics: Vec<Diagnostic>) {
        self.client
            .publish_diagnostics(uri.parse().unwrap(), diagnostics, None)
            .await;
    }

    // Optimization strategy: Call this only when build files change
    // TODO: implement the caller
    pub fn should_refresh_dependencies(
        project_root: &PathBuf,
        last_refresh: std::time::SystemTime,
    ) -> bool {
        let build_files = [
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "gradle.properties",
        ];

        build_files.iter().any(|file| {
            let path = project_root.join(file);
            path.exists()
                && path
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|modified| modified > last_refresh)
                    .unwrap_or(false)
        })
    }
}
