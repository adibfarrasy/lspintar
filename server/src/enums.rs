use crate::{
    as_lsp_location::AsLspLocation,
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata},
    },
};

#[derive(Clone)]
pub enum ResolvedSymbol {
    Project(Symbol),
    External(ExternalSymbol),
}

impl ResolvedSymbol {
    pub fn package_name(&self) -> &str {
        match self {
            ResolvedSymbol::Project(s) => &s.package_name,
            ResolvedSymbol::External(s) => &s.package_name,
        }
    }

    pub fn metadata(&self) -> &SymbolMetadata {
        match self {
            ResolvedSymbol::Project(s) => &s.metadata,
            ResolvedSymbol::External(s) => &s.metadata,
        }
    }

    pub fn fully_qualified_name(&self) -> &str {
        match self {
            ResolvedSymbol::Project(s) => &s.fully_qualified_name,
            ResolvedSymbol::External(s) => &s.fully_qualified_name,
        }
    }
}

impl AsLspLocation for ResolvedSymbol {
    fn as_lsp_location(&self) -> Option<tower_lsp::lsp_types::Location> {
        match self {
            ResolvedSymbol::Project(s) => s.as_lsp_location(),
            ResolvedSymbol::External(s) => s.as_lsp_location(),
        }
    }
}
