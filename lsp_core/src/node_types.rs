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
}
