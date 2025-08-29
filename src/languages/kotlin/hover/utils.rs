/// Partition modifiers into annotations and other modifiers for Kotlin (like Java)
/// Annotations start with '@', everything else is considered a modifier
pub fn partition_modifiers(modifiers: &str) -> (Vec<String>, Vec<String>) {
    modifiers
        .split_whitespace()
        .map(|s| s.to_string())
        .partition(|m| m.starts_with('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_modifiers() {
        assert_eq!(
            partition_modifiers("@Deprecated public abstract"),
            (vec!["@Deprecated".to_string()], vec!["public".to_string(), "abstract".to_string()])
        );

        assert_eq!(
            partition_modifiers("private data class"),
            (vec![], vec!["private".to_string(), "data".to_string(), "class".to_string()])
        );

        assert_eq!(
            partition_modifiers("@Override @JvmStatic internal sealed"),
            (vec!["@Override".to_string(), "@JvmStatic".to_string()], vec!["internal".to_string(), "sealed".to_string()])
        );

        assert_eq!(
            partition_modifiers("open"),
            (vec![], vec!["open".to_string()])
        );

        assert_eq!(
            partition_modifiers("@Suppress protected"),
            (vec!["@Suppress".to_string()], vec!["protected".to_string()])
        );
    }
}