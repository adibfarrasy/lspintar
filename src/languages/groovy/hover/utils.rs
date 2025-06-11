pub fn partition_modifiers(modifiers: String) -> (Vec<String>, Vec<String>) {
    modifiers
        .split_whitespace()
        .map(|s| s.to_string())
        .partition(|m| m.starts_with('@'))
}
