use tower_lsp::lsp_types::Location;

pub trait AsLspLocation {
    fn as_lsp_location(&self) -> Option<Location>;
}
