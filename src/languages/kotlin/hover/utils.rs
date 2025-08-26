/// Partition modifiers into access modifiers and other modifiers for Kotlin
/// Access modifiers: public, internal, protected, private
/// Other modifiers: abstract, final, open, sealed, data, inner, inline, suspend, etc.
pub fn partition_modifiers(modifiers: &str) -> (String, String) {
    let access_modifiers = ["public", "internal", "protected", "private"];
    let mut access = Vec::new();
    let mut other = Vec::new();

    for modifier in modifiers.split_whitespace() {
        if access_modifiers.contains(&modifier) {
            access.push(modifier);
        } else {
            other.push(modifier);
        }
    }

    (access.join(" "), other.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_modifiers() {
        assert_eq!(
            partition_modifiers("public abstract"),
            ("public".to_string(), "abstract".to_string())
        );

        assert_eq!(
            partition_modifiers("private data class"),
            ("private".to_string(), "data class".to_string())
        );

        assert_eq!(
            partition_modifiers("internal sealed"),
            ("internal".to_string(), "sealed".to_string())
        );

        assert_eq!(
            partition_modifiers("open"),
            ("".to_string(), "open".to_string())
        );

        assert_eq!(
            partition_modifiers("protected"),
            ("protected".to_string(), "".to_string())
        );
    }
}