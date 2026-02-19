use tower_lsp::lsp_types::{Hover, Location};

pub trait AsLspLocation {
    fn as_lsp_location(&self) -> Option<Location>;
}

pub trait AsLspHover {
    fn as_lsp_hover(&self) -> Option<Hover>;
}
