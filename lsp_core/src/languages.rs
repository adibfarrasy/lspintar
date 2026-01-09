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
            Language::Java => write!(f, "Java"),
            Language::Groovy => write!(f, "Groovy"),
            Language::Kotlin => write!(f, "Kotlin"),
        }
    }
}
