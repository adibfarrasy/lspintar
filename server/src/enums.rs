use tower_lsp::lsp_types::{
    Hover, HoverContents, Location, MarkupContent, MarkupKind, Position, Range, Url,
};

use crate::{
    lsp_convert::{AsLspHover, AsLspLocation},
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata},
    },
};

#[derive(Clone)]
pub enum ResolvedSymbol {
    Project(Symbol),
    External(ExternalSymbol),
    Local {
        uri: Url,
        position: Position,
        name: String,
        var_type: Option<String>,
    },
}

impl ResolvedSymbol {
    pub fn package_name(&self) -> Option<&str> {
        match self {
            ResolvedSymbol::Project(s) => Some(&s.package_name),
            ResolvedSymbol::External(s) => Some(&s.package_name),
            ResolvedSymbol::Local { .. } => None,
        }
    }

    pub fn metadata(&self) -> Option<&SymbolMetadata> {
        match self {
            ResolvedSymbol::Project(s) => Some(&s.metadata),
            ResolvedSymbol::External(s) => Some(&s.metadata),
            ResolvedSymbol::Local { .. } => None,
        }
    }

    pub fn fully_qualified_name(&self) -> Option<&str> {
        match self {
            ResolvedSymbol::Project(s) => Some(&s.fully_qualified_name),
            ResolvedSymbol::External(s) => Some(&s.fully_qualified_name),
            ResolvedSymbol::Local { .. } => None,
        }
    }
}

impl AsLspLocation for ResolvedSymbol {
    fn as_lsp_location(&self) -> Option<tower_lsp::lsp_types::Location> {
        match self {
            ResolvedSymbol::Project(s) => s.as_lsp_location(),
            ResolvedSymbol::External(s) => s.as_lsp_location(),
            ResolvedSymbol::Local { uri, position, .. } => {
                Some(Location::new(uri.clone(), Range::new(*position, *position)))
            }
        }
    }
}

impl AsLspHover for ResolvedSymbol {
    fn as_lsp_hover(&self) -> Option<tower_lsp::lsp_types::Hover> {
        match self {
            ResolvedSymbol::Project(s) => s.as_lsp_hover(),
            ResolvedSymbol::External(s) => s.as_lsp_hover(),
            ResolvedSymbol::Local { name, var_type, .. } => {
                let value = match var_type {
                    Some(t) => format!("```\n{} {}\n```", t, name),
                    None => format!("```\n{}\n```", name),
                };
                Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range: None,
                })
            }
        }
    }
}
