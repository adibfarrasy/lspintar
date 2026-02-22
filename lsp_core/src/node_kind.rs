use std::fmt::{self, Display, Formatter};

use tower_lsp::lsp_types::CompletionItemKind;

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Class,
    Interface,
    Function,
    Field,
    Enum,
    Annotation,
}

impl Display for NodeKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            NodeKind::Class => write!(f, "Class"),
            NodeKind::Interface => write!(f, "Interface"),
            NodeKind::Function => write!(f, "Function"),
            NodeKind::Field => write!(f, "Field"),
            NodeKind::Enum => write!(f, "Enum"),
            NodeKind::Annotation => write!(f, "Annotation"),
        }
    }
}

impl NodeKind {
    pub fn from_string(name: &str) -> Option<Self> {
        match name {
            "Class" => Some(NodeKind::Class),
            "Interface" => Some(NodeKind::Interface),
            "Function" => Some(NodeKind::Function),
            "Field" => Some(NodeKind::Field),
            "Enum" => Some(NodeKind::Enum),
            "Annotation" => Some(NodeKind::Annotation),
            _ => None,
        }
    }

    pub fn keyword(&self, file_type: &str) -> Option<&'static str> {
        match self {
            NodeKind::Class => Some("class"),
            NodeKind::Interface => match file_type {
                "kt" => Some("interface"),
                _ => Some("interface"),
            },
            NodeKind::Function => match file_type {
                "kt" => Some("fun"),
                _ => None,
            },
            NodeKind::Enum => match file_type {
                "kt" => Some("enum class"),
                _ => Some("enum"),
            },
            NodeKind::Annotation => Some("@interface"),
            NodeKind::Field => None, // just show type + name
        }
    }

    pub fn to_lsp_kind(&self) -> Option<CompletionItemKind> {
        match self {
            NodeKind::Class => Some(CompletionItemKind::CLASS),
            NodeKind::Interface => Some(CompletionItemKind::INTERFACE),
            NodeKind::Function => Some(CompletionItemKind::FUNCTION),
            NodeKind::Field => Some(CompletionItemKind::FIELD),
            NodeKind::Enum => Some(CompletionItemKind::ENUM),
            NodeKind::Annotation => Some(CompletionItemKind::CLASS),
        }
    }
}
