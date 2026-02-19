use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone)]
pub enum Language {
    Java,
    Groovy,
    Kotlin,
}

impl Display for Language {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Language::Java => write!(f, "java"),
            Language::Groovy => write!(f, "groovy"),
            Language::Kotlin => write!(f, "kotlin"),
        }
    }
}
