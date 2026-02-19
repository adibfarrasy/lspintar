use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Class,
    Interface,
    Function,
    Field,
    Enum,
    Annotation,
}

impl Display for NodeType {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            NodeType::Class => write!(f, "Class"),
            NodeType::Interface => write!(f, "Interface"),
            NodeType::Function => write!(f, "Function"),
            NodeType::Field => write!(f, "Field"),
            NodeType::Enum => write!(f, "Enum"),
            NodeType::Annotation => write!(f, "Annotation"),
        }
    }
}

impl NodeType {
    pub fn from_string(name: &str) -> Option<Self> {
        match name {
            "Class" => Some(NodeType::Class),
            "Interface" => Some(NodeType::Interface),
            "Function" => Some(NodeType::Function),
            "Field" => Some(NodeType::Field),
            "Enum" => Some(NodeType::Enum),
            "Annotation" => Some(NodeType::Annotation),
            _ => None,
        }
    }

    pub fn keyword(&self, file_type: &str) -> Option<&'static str> {
        match self {
            NodeType::Class => Some("class"),
            NodeType::Interface => match file_type {
                "kt" => Some("interface"),
                _ => Some("interface"),
            },
            NodeType::Function => match file_type {
                "kt" => Some("fun"),
                _ => None,
            },
            NodeType::Enum => match file_type {
                "kt" => Some("enum class"),
                _ => Some("enum"),
            },
            NodeType::Annotation => Some("@interface"),
            NodeType::Field => None, // just show type + name
        }
    }
}
