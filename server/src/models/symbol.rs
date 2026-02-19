use lsp_core::{node_types::NodeType, util::strip_comment_signifiers};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, types::Json};
use tower_lsp::lsp_types::{
    Hover, HoverContents, Location, MarkupContent, MarkupKind, Position, Range, Url,
};

use crate::lsp_convert::{AsLspHover, AsLspLocation};

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct Symbol {
    pub id: Option<i64>,
    pub vcs_branch: String,
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<String>>,
}

impl AsLspLocation for Symbol {
    fn as_lsp_location(&self) -> Option<Location> {
        let uri = Url::from_file_path(&self.file_path).ok()?;
        Some(Location {
            uri: uri,
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
        let mut parts = Vec::new();
        parts.push(format!("```{}", self.file_type));
        if !self.package_name.is_empty() {
            parts.push(format!("package {}", self.package_name));
            parts.push(String::new());
        }
        if let Some(annotations) = &self.metadata.annotations {
            for annotation in annotations {
                if !annotation.is_empty() {
                    parts.push(annotation.clone());
                }
            }
        }

        let node_type = NodeType::from_string(&self.symbol_type);
        let modifiers = self.modifiers.iter().cloned().collect::<Vec<_>>().join(" ");
        let mut signature_line = String::new();

        if !modifiers.is_empty() {
            signature_line.push_str(&modifiers);
            signature_line.push(' ');
        }

        match node_type {
            Some(NodeType::Function) => {
                if let Some(kw) = NodeType::Function.keyword(&self.file_type) {
                    signature_line.push_str(kw);
                    signature_line.push(' ');
                }
                if let Some(ret) = &self.metadata.return_type {
                    signature_line.push_str(ret);
                    signature_line.push(' ');
                }
                signature_line.push_str(&self.short_name);
            }
            Some(NodeType::Field) => {
                if let Some(ret) = &self.metadata.return_type {
                    signature_line.push_str(ret);
                    signature_line.push(' ');
                }
                signature_line.push_str(&self.short_name);
            }
            Some(ref nt) => {
                if let Some(kw) = nt.keyword(&self.file_type) {
                    signature_line.push_str(kw);
                    signature_line.push(' ');
                }
                signature_line.push_str(&self.short_name);
            }
            None => {
                signature_line.push_str(&self.short_name);
            }
        }

        parts.push(signature_line);

        if let Some(params) = &self.metadata.parameters {
            if !params.is_empty() {
                let format_param = |p: &SymbolParameter| {
                    let mut s = match &p.type_name {
                        Some(t) => format!("{} {}", t, p.name),
                        None => p.name.clone(),
                    };
                    if let Some(default) = &p.default_value {
                        s.push_str(&format!(" = {}", default));
                    }
                    s
                };
                if params.len() > 3 {
                    parts.push("(".to_string());
                    for param in params {
                        parts.push(format!("    {},", format_param(param)));
                    }
                    parts.push(")".to_string());
                } else {
                    let params_str = params
                        .iter()
                        .map(format_param)
                        .collect::<Vec<_>>()
                        .join(", ");
                    parts.push(format!("({})", params_str));
                }
            }
        }

        if self.metadata.documentation.is_some() {
            parts.push(String::new());
            parts.push("---".to_string());
        }
        parts.push("```".to_string());
        if let Some(doc) = &self.metadata.documentation {
            if !doc.is_empty() {
                parts.push(strip_comment_signifiers(doc));
            }
        }
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: parts.join("\n"),
            }),
            range: None,
        })
    }
}
