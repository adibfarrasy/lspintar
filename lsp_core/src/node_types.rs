use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Class,
    Interface,
    Function,
    Field,
    Enum,
}

impl Display for NodeType {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            NodeType::Class => write!(f, "Class"),
            NodeType::Interface => write!(f, "Interface"),
            NodeType::Function => write!(f, "Function"),
            NodeType::Field => write!(f, "Field"),
            NodeType::Enum => write!(f, "Enum"),
        }
    }
}
