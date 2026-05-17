use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub enum Language {
    Java,
    Groovy,
    Kotlin,
    Scala,
}

impl Display for Language {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Language::Java => write!(f, "java"),
            Language::Groovy => write!(f, "groovy"),
            Language::Kotlin => write!(f, "kotlin"),
            Language::Scala => write!(f, "scala"),
        }
    }
}
