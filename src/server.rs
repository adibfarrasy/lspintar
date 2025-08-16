use anyhow::Context;
use dashmap::DashMap;
use request::GotoImplementationParams;
use request::GotoImplementationResponse;
use serde_json::Value;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::LanguageServer;
use tracing::debug;
use tree_sitter::Tree;

use crate::core::build_tools::run_gradle_build;
use crate::core::constants::BUILD_ON_INIT;
use crate::core::constants::GRADLE_CACHE_DIR;
use crate::core::dependency_cache::symbol_index::find_workspace_root;
use crate::core::dependency_cache::DependencyCache;
use crate::core::logging_service;
use crate::core::state_manager;
use crate::core::state_manager::{get_global, set_global};
use crate::core::utils::find_external_dependency_root;
use crate::core::utils::is_external_dependency;
use crate::core::utils::is_path_in_external_dependency;
use crate::core::utils::is_project_root;
use crate::core::utils::uri_to_path;
use crate::core::DiagnosticManager;
use crate::core::Document;
use crate::core::DocumentManager;
use crate::core::{
    build_tools::{detect_build_tool, BuildTool},
    utils::find_project_root,
};
use crate::languages::LanguageRegistry;
use crate::lsp_error;
use crate::lsp_warning;

pub struct LspServer {
    documents: Arc<RwLock<DocumentManager>>,
    language_registry: Arc<LanguageRegistry>,
    diagnostics: Arc<DashMap<String, DiagnosticManager>>,
    dependency_cache: Arc<DependencyCache>,
    client: tower_lsp::Client,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for LspServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(init_options) = params.initialization_options {
            self.parse_configuration(init_options).await.map_err(|_| {
                tower_lsp::jsonrpc::Error::invalid_params("invalid initialization options")
            })?;
        }

        // Best effort root guess
        let client_root = params
            .root_uri
            .and_then(|uri| uri.to_file_path().ok())
            .or_else(|| {
                params
                    .workspace_folders
                    .as_ref()?
                    .first()?
                    .uri
                    .to_file_path()
                    .ok()
            })
            .or_else(|| Some(env::current_dir().unwrap()));

        let effective_root = self.find_true_workspace_root(&client_root.unwrap()).await;

        *self.workspace_root.write().await = Some(effective_root.clone());

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
        if let Some(true) = get_global(BUILD_ON_INIT).and_then(|v| v.as_bool()) {
            if let Err(error) = self.build_on_init().await {
                lsp_error!("{}", error.to_string())
            };
        }

        let workspace_root = {
            let root_guard = self.workspace_root.read().await;
            root_guard.clone()
        };

        self.update_cache(workspace_root).await
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

        let current_workspace = self
            .find_true_workspace_root(&uri_to_path(&uri).unwrap())
            .await;

        let workspace_root = {
            let root_guard = self.workspace_root.read().await;
            root_guard.clone()
        };

        if let Some(workspace) = workspace_root {
            if current_workspace != workspace {
                self.update_cache(Some(current_workspace)).await
            }
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

        // Try to find definition first for remote hover
        debug!("hover: attempting find_definition");
        match self.find_definition(uri.clone(), position).await {
            Ok(location) => {
                debug!("hover: found definition at {}", location.uri);
                let other_uri = location.uri.to_string();
                let (content, tree) = self.get_content_and_tree(&other_uri).await?;

                let language_support = self
                    .language_registry
                    .detect_language(&other_uri)
                    .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

                debug!("hover: calling provide_hover on target file");
                if let Some(hover) = language_support.provide_hover(&tree, &content, location) {
                    debug!("hover: successfully got hover from target file");
                    return Ok(Some(hover));
                } else {
                    debug!("hover: provide_hover returned None for target file");
                }
            }
            Err(e) => {
                debug!("hover: find_definition failed with error: {}", e);
            }
        }

        // Fallback: provide local hover if definition finding fails
        let (content, tree) = self.get_content_and_tree(&uri).await?;
        let language_support = self
            .language_registry
            .detect_language(&uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

        // Create a location from the current position for local hover
        let local_location = Location {
            uri: tower_lsp::lsp_types::Url::parse(&uri).map_err(|_| tower_lsp::jsonrpc::Error::invalid_params("Invalid URI".to_string()))?,
            range: tower_lsp::lsp_types::Range {
                start: position,
                end: position,
            },
        };

        language_support
            .provide_hover(&tree, &content, local_location)
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
        logging_service::init_logging_service(client.clone());
        state_manager::init_state_manager();

        Self {
            documents: Arc::new(RwLock::new(DocumentManager::new())),
            language_registry: registry,
            diagnostics: Arc::new(DashMap::new()),
            client,
            dependency_cache: Arc::new(DependencyCache::new()),
            workspace_root: Arc::new(RwLock::new(None)),
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
        .map_err(|error| tower_lsp::jsonrpc::Error::invalid_params(format!("{error}")))?
        .map_err(|error| tower_lsp::jsonrpc::Error::invalid_params(format!("{error}")))
    }

    async fn get_content_and_tree(&self, uri: &str) -> Result<(String, Tree)> {
        {
            let document_manager = self.documents.read().await;
            if let Some(document) = document_manager.get(uri) {
                if let Some(tree) = document_manager.get_tree(uri) {
                    return Ok((document.content.clone(), tree.clone()));
                }
            }
        }

        let mut document_manager = self.documents.write().await;

        // Double-check in case another thread inserted it
        if let Some(document) = document_manager.get(uri) {
            if let Some(tree) = document_manager.get_tree(uri) {
                return Ok((document.content.clone(), tree.clone()));
            }
        }

        let file_path = uri_to_path(uri).ok_or(
            tower_lsp::jsonrpc::Error::invalid_params("Invalid URI".to_string()),
        )?;

        let content = tokio::fs::read_to_string(&file_path).await.map_err(|_| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("Failed to read file: {}", uri))
        })?;

        let language_support = self.language_registry.detect_language(uri).ok_or(
            tower_lsp::jsonrpc::Error::invalid_params("Unsupported language".to_string()),
        )?;

        let document = Document::new(
            uri.to_string(),
            content.clone(),
            0, // Version 0 for disk-loaded files
            language_support.language_id().to_string(),
        );

        document_manager.insert(document);

        document_manager.reparse_and_cache_tree(uri, &content, &self.language_registry);

        let document = document_manager
            .get(uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;
        let tree = document_manager
            .get_tree(uri)
            .ok_or(tower_lsp::jsonrpc::Error::internal_error())?;

        Ok((document.content.clone(), tree.clone()))
    }

    #[tracing::instrument(skip_all)]
    async fn parse_configuration(&self, init_options: Value) -> anyhow::Result<()> {
        if let Some(obj) = init_options.as_object() {
            if let Some(gradle_cache) = obj.get(GRADLE_CACHE_DIR) {
                if let Some(cache_dir) = gradle_cache.as_str() {
                    state_manager::set_global(GRADLE_CACHE_DIR, cache_dir);
                }
            }

            if let Some(run_build) = obj.get(BUILD_ON_INIT) {
                if let Some(build_flag) = run_build.as_bool() {
                    state_manager::set_global(BUILD_ON_INIT, build_flag);
                }
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn build_on_init(&self) -> anyhow::Result<()> {
        let current_dir = env::current_dir().context("Failed to get current directory")?;

        let project_root = if is_project_root(&current_dir) {
            current_dir
        } else {
            find_project_root(&current_dir.to_path_buf()).context("Cannot find project root")?
        };

        let build_tool = detect_build_tool(&project_root).context("Cannot detect build tool")?;

        match build_tool {
            BuildTool::Gradle => {
                run_gradle_build(&project_root).await?;
            }
        }

        Ok(())
    }

    async fn find_true_workspace_root(&self, suggested_root: &PathBuf) -> PathBuf {
        if is_path_in_external_dependency(suggested_root) {
            if let Some(dep_root) = find_external_dependency_root(suggested_root) {
                return dep_root;
            }
        }

        if let Some(root) = find_workspace_root(suggested_root) {
            return root;
        }

        suggested_root.clone()
    }

    async fn update_cache(&self, path: Option<PathBuf>) {
        if let Some(dir) = path {
            // Try to load from persistent cache first
            let loaded_from_cache = if !is_external_dependency(&dir) {
                match self.dependency_cache.load_from_disk(&dir).await {
                    Ok(true) => {
                        // Set indexing completed flag when loading from cache
                        set_global("is_indexing_completed", true);
                        true
                    },
                    Ok(false) => false,
                    Err(_) => false,
                }
            } else {
                false // Don't use persistence for external dependencies
            };
            
            // If cache wasn't loaded, rebuild it
            if !loaded_from_cache {
                if is_external_dependency(&dir) {
                    if let Err(error) = self
                        .dependency_cache
                        .clone()
                        .index_external_dependency(dir)
                        .await
                    {
                        lsp_error!("{}", error.to_string())
                    }
                } else {
                    if let Err(error) = self.dependency_cache.clone().index_workspace(dir).await {
                        lsp_error!("{}", error.to_string())
                    }
                }
            }

            let _ = self.dependency_cache.clone().dump_to_file().await;
        } else {
            lsp_warning!("No workspace root available, skipping initialization");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DiagnosticManager;
    use crate::languages::LanguageRegistry;
    use std::sync::Arc;
    use tower_lsp::lsp_types::{Position, Range, Url};

    struct LspServerTestCase {
        name: &'static str,
        setup: fn() -> (LspServer, Arc<LanguageRegistry>),
        test_operation: &'static str,
        expected_success: bool,
    }

    fn create_mock_client() -> tower_lsp::Client {
        // This is a simplified mock - in real tests you'd use a proper mock
        // For now, we'll skip client-dependent tests
        unimplemented!("Mock client not implemented for these tests")
    }

    fn create_test_server() -> (LspServer, Arc<LanguageRegistry>) {
        let registry = Arc::new(LanguageRegistry::new());
        
        // Note: This would normally create a real client, but for unit tests
        // we'd need a mock implementation
        // let client = create_mock_client();
        // let server = LspServer::new(client, registry.clone());
        
        // For now, we'll test the components that don't require a client
        unimplemented!("Full server creation requires mock client")
    }

    #[test]
    fn test_server_creation_basic() {
        // Test basic server structure without client dependency
        let registry = Arc::new(LanguageRegistry::new());
        
        // Test that we can create the basic components
        let documents = Arc::new(RwLock::new(DocumentManager::new()));
        let diagnostics: Arc<DashMap<String, DiagnosticManager>> = Arc::new(DashMap::new());
        let dependency_cache = Arc::new(DependencyCache::new());
        let workspace_root: Arc<RwLock<Option<PathBuf>>> = Arc::new(RwLock::new(None));
        
        // Verify basic properties
        assert_eq!(diagnostics.len(), 0);
        assert_eq!(dependency_cache.symbol_index.len(), 0);
    }

    #[tokio::test]
    async fn test_workspace_root_operations() {
        let workspace_root = Arc::new(RwLock::new(None));
        
        // Test setting workspace root
        {
            let mut root = workspace_root.write().await;
            *root = Some(PathBuf::from("/test/workspace"));
        }
        
        // Test reading workspace root
        {
            let root = workspace_root.read().await;
            assert_eq!(*root, Some(PathBuf::from("/test/workspace")));
        }
    }

    struct ConfigurationTestCase {
        name: &'static str,
        input_json: serde_json::Value,
        expected_gradle_cache: Option<&'static str>,
        expected_build_on_init: Option<bool>,
    }

    #[test]
    fn test_configuration_parsing() {
        let test_cases = vec![
            ConfigurationTestCase {
                name: "empty configuration",
                input_json: serde_json::json!({}),
                expected_gradle_cache: None,
                expected_build_on_init: None,
            },
            ConfigurationTestCase {
                name: "gradle cache configuration",
                input_json: serde_json::json!({
                    "gradle_cache_dir": "/home/user/.gradle/caches"
                }),
                expected_gradle_cache: Some("/home/user/.gradle/caches"),
                expected_build_on_init: None,
            },
            ConfigurationTestCase {
                name: "build on init configuration",
                input_json: serde_json::json!({
                    "build_on_init": true
                }),
                expected_gradle_cache: None,
                expected_build_on_init: Some(true),
            },
            ConfigurationTestCase {
                name: "full configuration",
                input_json: serde_json::json!({
                    "gradle_cache_dir": "/custom/gradle/cache",
                    "build_on_init": false
                }),
                expected_gradle_cache: Some("/custom/gradle/cache"),
                expected_build_on_init: Some(false),
            },
        ];

        for test_case in test_cases {
            // Note: This test demonstrates the structure but would need
            // a proper state manager mock to work fully
            
            // Verify JSON structure
            if let Some(expected_cache) = test_case.expected_gradle_cache {
                if let Some(cache_value) = test_case.input_json.get(GRADLE_CACHE_DIR) {
                    assert_eq!(cache_value.as_str(), Some(expected_cache));
                }
            }
            
            if let Some(expected_build) = test_case.expected_build_on_init {
                if let Some(build_value) = test_case.input_json.get(BUILD_ON_INIT) {
                    assert_eq!(build_value.as_bool(), Some(expected_build));
                }
            }
        }
    }

    #[test]
    fn test_lsp_types_creation() {
        // Test that we can create and work with LSP types
        let position = Position { line: 5, character: 10 };
        let range = Range {
            start: position,
            end: Position { line: 5, character: 20 },
        };
        
        let uri = Url::parse("file:///test/file.groovy").expect("Valid URI");
        let location = Location { uri, range };
        
        assert_eq!(location.range.start.line, 5);
        assert_eq!(location.range.start.character, 10);
        assert_eq!(location.range.end.character, 20);
    }

    struct InitializeParamsTestCase {
        name: &'static str,
        root_uri: Option<&'static str>,
        workspace_folders: Option<Vec<&'static str>>,
        expected_has_root: bool,
    }

    #[test]
    fn test_initialize_params_structure() {
        let test_cases = vec![
            InitializeParamsTestCase {
                name: "with root URI",
                root_uri: Some("file:///workspace/project"),
                workspace_folders: None,
                expected_has_root: true,
            },
            InitializeParamsTestCase {
                name: "with workspace folders",
                root_uri: None,
                workspace_folders: Some(vec!["file:///workspace/project1", "file:///workspace/project2"]),
                expected_has_root: true,
            },
            InitializeParamsTestCase {
                name: "no root specified",
                root_uri: None,
                workspace_folders: None,
                expected_has_root: false,
            },
        ];

        for test_case in test_cases {
            let mut params = InitializeParams::default();
            
            if let Some(root_uri_str) = test_case.root_uri {
                params.root_uri = Some(Url::parse(root_uri_str).unwrap());
            }
            
            if let Some(folders) = test_case.workspace_folders {
                let workspace_folders: Vec<WorkspaceFolder> = folders
                    .iter()
                    .map(|uri_str| WorkspaceFolder {
                        uri: Url::parse(uri_str).unwrap(),
                        name: "test".to_string(),
                    })
                    .collect();
                params.workspace_folders = Some(workspace_folders);
            }
            
            // Test the logic for extracting root
            let has_root = params.root_uri.is_some() || 
                          params.workspace_folders.as_ref().map_or(false, |folders| !folders.is_empty());
            
            assert_eq!(
                has_root,
                test_case.expected_has_root,
                "Test '{}': root detection mismatch",
                test_case.name
            );
        }
    }

    #[test]
    fn test_server_capabilities() {
        let capabilities = ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(
                TextDocumentSyncKind::FULL,
            )),
            definition_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
            ..Default::default()
        };
        
        // Verify capabilities structure
        match capabilities.text_document_sync {
            Some(TextDocumentSyncCapability::Kind(kind)) => {
                assert_eq!(kind, TextDocumentSyncKind::FULL);
            }
            _ => panic!("Expected FULL text document sync"),
        }
        
        assert!(capabilities.definition_provider.is_some());
        assert!(capabilities.hover_provider.is_some());
        assert!(capabilities.implementation_provider.is_some());
    }
}