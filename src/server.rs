use dashmap::DashMap;
use request::GotoImplementationParams;
use request::GotoImplementationResponse;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;
use tree_sitter::Tree;

use crate::core::dependency_cache::DependencyCache;
use crate::core::DiagnosticManager;
use crate::core::Document;
use crate::core::DocumentManager;
use crate::languages::LanguageRegistry;

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
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "LSP server initialized")
            .await;

        if let Err(error) = self.dependency_cache.index_workspace().await {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("An error occurred: {}", error.to_string()),
                )
                .await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let text_document = params.text_document;
        let uri = text_document.uri.to_string();

        let language_support = self.language_registry.detect_language(&uri);
        if language_support.is_none() {
            return;
        }

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

        self.diagnostics
            .entry(uri.clone())
            .or_insert_with(|| {
                DiagnosticManager::new(self.client.clone(), self.language_registry.clone())
            })
            .request_diagnostics(uri, content, version);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        {
            let mut document = self.documents.write().await;
            document.remove(&uri);
        }

        // Clear diagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;

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

        let result = self.find_definition(uri, position).await;

        match result {
            Ok(location) => Ok(Some(GotoDefinitionResponse::Scalar(location))),
            Err(error) => Err(tower_lsp::jsonrpc::Error::invalid_params(error.to_string())),
        }
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let language_support = self
            .language_registry
            .detect_language(&uri)
            .ok_or(tower_lsp::jsonrpc::Error::invalid_request())?;

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

            (document.content.clone(), tree.clone())
        };

        let cache = self.dependency_cache.clone();

        let result = tokio::task::spawn_blocking(move || {
            language_support.find_implementation(&tree, &content, position, cache)
        })
        .await
        .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())?;

        match result {
            Ok(locations) if !locations.is_empty() => {
                Ok(Some(GotoImplementationResponse::Array(locations)))
            }
            Ok(_) => Ok(None),
            Err(error) => Err(tower_lsp::jsonrpc::Error::invalid_params(error.to_string())),
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let location = self.find_definition(uri.clone(), position).await?;

        let (content, tree) = self.get_content_and_tree(&uri).await?;

        let language_support = self
            .language_registry
            .detect_language(&uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

        language_support
            .provide_hover(&tree, &content, location)
            .ok_or(tower_lsp::jsonrpc::Error::invalid_request())
            .map(Some)
    }

    // Future features
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

    async fn find_definition(&self, uri: String, position: Position) -> Result<Location> {
        let (content, tree) = self.get_content_and_tree(&uri).await?;
        let cache = self.dependency_cache.clone();

        let language_support = self
            .language_registry
            .detect_language(&uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

        tokio::task::spawn_blocking(move || {
            language_support.find_definition(&tree, &content, position, &uri, cache)
        })
        .await
        .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())?
        .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())
    }

    async fn get_content_and_tree(&self, uri: &str) -> Result<(String, Tree)> {
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

        Ok((document.content.clone(), tree.clone()))
    }
}
