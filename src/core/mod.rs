pub mod build_tools;
pub mod constants;
pub mod dependency_cache;
pub mod diagnostic_manager;
pub mod document_manager;
pub mod jar_utils;
pub mod logging_service;
pub mod persistence;
pub mod state_manager;
pub mod symbols;
pub mod utils;

// New shared functionality modules
pub mod definition;
pub mod cross_language;
pub mod registry;

pub use diagnostic_manager::DiagnosticManager;
pub use document_manager::{Document, DocumentManager};
pub use registry::LanguageRegistry;
