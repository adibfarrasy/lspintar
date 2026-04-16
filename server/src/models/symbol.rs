use serde::{Deserialize, Serialize};
use sqlx::{FromRow, types::Json};
use tower_lsp::lsp_types::{
    Hover, HoverContents, Location, MarkupContent, MarkupKind, Position, Range, Url,
};

use crate::{
    lsp_convert::{AsLspHover, AsLspLocation},
    models::util::build_hover_parts,
};

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct Symbol {
    pub id: Option<i64>,
    pub short_name: String,
    pub package_name: String,
    pub fully_qualified_name: String,
    pub parent_name: Option<String>,
    pub file_path: String,
    pub file_type: String,
    pub symbol_type: String,

    #[sqlx(json)]
    pub modifiers: Json<Vec<String>>,

    pub line_start: i64,
    pub line_end: i64,
    pub char_start: i64,
    pub char_end: i64,

    pub ident_line_start: i64,
    pub ident_line_end: i64,
    pub ident_char_start: i64,
    pub ident_char_end: i64,

    #[sqlx(json)]
    pub metadata: Json<SymbolMetadata>,
    pub last_modified: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolParameter {
    pub name: String,
    pub type_name: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<SymbolParameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,

    /// Generic return type with type variables preserved, e.g. "E" or "List<E>".
    /// Derived from the JVM Signature attribute; absent when the method is not generic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic_return_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<String>>,

    /// Ordered list of type parameter names declared on this class/interface,
    /// e.g. ["E"] for List<E>, ["K", "V"] for Map<K,V>.
    /// Derived from the JVM Signature attribute; absent for non-generic types.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_params: Option<Vec<String>>,

    /// Generic parameter types for this method with type variables preserved,
    /// e.g. ["Consumer<T>"] for forEach or ["Function1<T, Unit>"] for Kotlin forEach.
    /// Derived from the JVM Signature attribute; absent when the method is not generic
    /// or has no parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic_param_types: Option<Vec<String>>,

    /// Ordered list of method-level type parameter names declared on this method,
    /// e.g. ["R"] for `<R> R map(Function<T, R>)`.
    /// Derived from the JVM Signature attribute; absent for non-generic methods.
    /// Used to build call-site bindings when explicit type args appear at the call site.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_type_params: Option<Vec<String>>,
}

impl AsLspLocation for Symbol {
    fn as_lsp_location(&self) -> Option<Location> {
        let uri = Url::from_file_path(&self.file_path).ok()?;
        Some(Location {
            uri,
            range: Range {
                start: Position {
                    line: self.ident_line_start as u32,
                    character: self.ident_char_start as u32,
                },
                end: Position {
                    line: self.ident_line_end as u32,
                    character: self.ident_char_end as u32,
                },
            },
        })
    }
}

impl AsLspHover for Symbol {
    fn as_lsp_hover(&self) -> Option<Hover> {
        let parts = build_hover_parts(
            &self.file_type,
            &self.package_name,
            &self.short_name,
            &self.symbol_type,
            &self.modifiers,
            &self.metadata,
        );
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: parts.join("\n"),
            }),
            range: None,
        })
    }
}
