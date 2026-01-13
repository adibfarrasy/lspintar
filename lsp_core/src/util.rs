pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// Only find direct import match
pub fn naive_resolve_fqn(name: &str, imports: Vec<String>) -> Option<String> {
    if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some(name)) {
        return Some(import.clone());
    }

    None
}
