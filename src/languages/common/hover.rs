/// Common hover functionality shared across all languages

#[derive(Debug, Clone)]
pub struct HoverSignature {
    pub package_name: Option<String>,
    pub language: String,
    pub annotations: Vec<String>,
    pub modifiers: Vec<String>,
    pub signature_line: String,
    pub inheritance: Option<String>,
    pub constructor_params: Vec<String>,
    pub documentation: Option<String>,
}

impl HoverSignature {
    pub fn new(language: &str) -> Self {
        Self {
            package_name: None,
            language: language.to_string(),
            annotations: Vec::new(),
            modifiers: Vec::new(),
            signature_line: String::new(),
            inheritance: None,
            constructor_params: Vec::new(),
            documentation: None,
        }
    }

    pub fn with_package(mut self, package: Option<String>) -> Self {
        self.package_name = package;
        self
    }

    pub fn with_annotations(mut self, annotations: Vec<String>) -> Self {
        self.annotations = annotations;
        self
    }

    pub fn with_modifiers(mut self, modifiers: Vec<String>) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub fn with_signature_line(mut self, signature: String) -> Self {
        self.signature_line = signature;
        self
    }

    pub fn with_inheritance(mut self, inheritance: Option<String>) -> Self {
        self.inheritance = inheritance;
        self
    }


    pub fn with_documentation(mut self, docs: Option<String>) -> Self {
        self.documentation = docs;
        self
    }

    /// Format the hover signature into a markdown string
    pub fn format(&self) -> String {
        let mut parts = Vec::new();

        // Start code block
        parts.push(format!("```{}", self.language));

        // Package name at the top with 'package' prefix
        if let Some(ref package) = self.package_name {
            if !package.is_empty() {
                parts.push(format!("package {}", package));
                parts.push("".to_string()); // Empty line after package
            }
        }

        // Annotations - each on separate lines, preserving multi-line format
        for annotation in &self.annotations {
            if !annotation.is_empty() {
                parts.push(annotation.clone());
            }
        }

        // Class/interface/object modifiers and identifiers on same line
        let mut signature_line = String::new();
        
        // Add modifiers
        if !self.modifiers.is_empty() {
            signature_line.push_str(&self.modifiers.join(" "));
            signature_line.push(' ');
        }

        // Add main signature
        signature_line.push_str(&self.signature_line);
        parts.push(signature_line);

        // Constructor parameters - handle special formatting
        if !self.constructor_params.is_empty() {
            // Split constructor args into multiple lines if more than 3 args
            let should_split_args = self.constructor_params.len() > 3;
            
            if should_split_args {
                // Each parameter on separate line with indentation and trailing commas
                parts.push("(".to_string());
                for param in &self.constructor_params {
                    if !param.trim().is_empty() {
                        parts.push(format!("    {},", param.trim().trim_end_matches(',')));
                    }
                }
                parts.push(")".to_string());
            } else {
                // Parameters inline when 3 or fewer
                let params_str = self.constructor_params.join(", ");
                parts.push(format!("({})", params_str));
            }
        }

        // Add inheritance line
        if let Some(ref inheritance) = self.inheritance {
            if !inheritance.is_empty() {
                let inheritance_line = if self.language == "kotlin" {
                    // For Kotlin, prefix with ':' 
                    if inheritance.starts_with(':') {
                        inheritance.clone()
                    } else {
                        format!(": {}", inheritance)
                    }
                } else {
                    // For Java/Groovy, use inheritance as-is (already has extends/implements)
                    inheritance.clone()
                };
                parts.push(inheritance_line);
            }
        }

        // Add empty line before separator if there's documentation
        if self.documentation.is_some() {
            parts.push("".to_string());
            parts.push("---".to_string());
        }

        // End code block
        parts.push("```".to_string());

        // Documentation with stripped comment signifiers after code block
        if let Some(ref docs) = self.documentation {
            if !docs.is_empty() {
                let cleaned_docs = strip_comment_signifiers(docs);
                parts.push(cleaned_docs);
            }
        }

        parts.join("\n")
    }
}

/// Common function to partition modifiers into annotations and other modifiers
/// Works for Java, Kotlin, Groovy - annotations start with '@'
pub fn partition_modifiers(modifiers: &str) -> (Vec<String>, Vec<String>) {
    modifiers
        .split_whitespace()
        .map(|s| s.to_string())
        .partition(|m| m.starts_with('@'))
}


/// Common function to parse parameters from raw text into individual parameter lines
/// This handles both constructor parameters and method parameters consistently
pub fn parse_parameters(param_text: &str) -> Vec<String> {
    if param_text.is_empty() {
        return Vec::new();
    }

    let content = param_text
        .trim_start_matches('(')
        .trim_end_matches(')');

    if content.trim().is_empty() {
        return Vec::new();
    }

    content
        .lines()
        .map(|line| line.trim().trim_end_matches(','))
        .filter(|line| {
            !line.is_empty() && 
            !line.chars().all(|c| c.is_whitespace() || c == ',' || c == ')' || c == '(')
        })
        .map(|line| line.to_string())
        .collect()
}

/// Format parameters according to the ≤3 vs >3 rule
/// - If ≤3 parameters: format inline as (param1, param2, param3)
/// - If >3 parameters: format multi-line with each parameter on separate line
pub fn format_parameters(params: &[String]) -> String {
    if params.is_empty() {
        return "()".to_string();
    }
    
    if params.len() <= 3 {
        // Inline format for 3 or fewer parameters
        format!("({})", params.join(", "))
    } else {
        // Multi-line format for more than 3 parameters
        let mut result = String::from("(\n");
        for param in params {
            result.push_str(&format!("    {},\n", param.trim().trim_end_matches(',')));
        }
        result.push(')');
        result
    }
}

/// Deduplicate modifiers to prevent repetition like "data data data"
/// This can happen when tree-sitter queries capture the same modifier multiple times
pub fn deduplicate_modifiers(modifiers: Vec<String>) -> Vec<String> {
    let mut unique_modifiers = Vec::new();
    let mut seen_modifiers = std::collections::HashSet::new();
    for modifier in modifiers {
        if seen_modifiers.insert(modifier.clone()) {
            unique_modifiers.push(modifier);
        }
    }
    unique_modifiers
}

/// Clean up inheritance/supertype text
pub fn format_inheritance(supertypes: &str) -> Option<String> {
    if supertypes.is_empty() {
        return None;
    }

    let cleaned = supertypes.replace('\n', ", ");
    Some(cleaned)
}

/// Strip comment signifiers from documentation text
/// Removes /*, *, */, // while preserving multi-line format
pub fn strip_comment_signifiers(docs: &str) -> String {
    let mut lines: Vec<String> = docs.lines()
        .map(|line| {
            let trimmed = line.trim();
            
            // Remove /* at start of line
            let without_start = if trimmed.starts_with("/**") {
                trimmed.strip_prefix("/**").unwrap_or(trimmed).trim()
            } else if trimmed.starts_with("/*") {
                trimmed.strip_prefix("/*").unwrap_or(trimmed).trim()
            } else { trimmed };
            
            // Remove */ at end of line
            let without_end = if without_start.ends_with("*/") {
                without_start.strip_suffix("*/").unwrap_or(without_start).trim()
            } else { without_start };
            
            // Remove leading * or // with more aggressive matching
            let without_prefix = if without_end.starts_with("* ") {
                without_end.strip_prefix("* ").unwrap_or(without_end)
            } else if without_end == "*" {
                // Handle standalone asterisks
                ""
            } else if without_end.starts_with("*") && without_end.len() > 1 {
                // Handle * immediately followed by content
                &without_end[1..]
            } else if without_end.starts_with("// ") {
                without_end.strip_prefix("// ").unwrap_or(without_end)
            } else if without_end.starts_with("//") {
                without_end.strip_prefix("//").unwrap_or(without_end)
            } else {
                without_end
            };
            
            without_prefix.trim().to_string()
        })
        .collect();
    
    // Remove empty lines at start and end
    while lines.first().map_or(false, |line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().map_or(false, |line| line.is_empty()) {
        lines.pop();
    }
    
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_modifiers() {
        let (annotations, modifiers) = partition_modifiers("@Override public static final");
        assert_eq!(annotations, vec!["@Override"]);
        assert_eq!(modifiers, vec!["public", "static", "final"]);
    }

    #[test]
    fn test_parse_parameters_empty() {
        assert_eq!(parse_parameters(""), Vec::<String>::new());
        assert_eq!(parse_parameters("()"), Vec::<String>::new());
    }

    #[test]
    fn test_parse_parameters_single() {
        let result = parse_parameters("(String name)");
        assert_eq!(result, vec!["String name"]);
    }

    #[test]
    fn test_parse_parameters_multiple() {
        let result = parse_parameters("(String name, int age, boolean active)");
        assert_eq!(result, vec!["String name", "int age", "boolean active"]);
    }

    #[test]
    fn test_parse_parameters_multiline() {
        let result = parse_parameters("(\n    String name,\n    int age,\n    boolean active\n)");
        assert_eq!(result, vec!["String name", "int age", "boolean active"]);
    }

    #[test]
    fn test_format_parameters_empty() {
        assert_eq!(format_parameters(&[]), "()");
    }

    #[test]
    fn test_format_parameters_inline_three() {
        let params = vec!["String name".to_string(), "int age".to_string(), "boolean active".to_string()];
        let result = format_parameters(&params);
        assert_eq!(result, "(String name, int age, boolean active)");
    }

    #[test]
    fn test_format_parameters_multiline_four() {
        let params = vec![
            "String name".to_string(),
            "int age".to_string(), 
            "boolean active".to_string(),
            "Date created".to_string()
        ];
        let result = format_parameters(&params);
        let expected = "(\n    String name,\n    int age,\n    boolean active,\n    Date created,\n)";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_deduplicate_modifiers() {
        let modifiers = vec!["data".to_string(), "public".to_string(), "data".to_string(), "final".to_string()];
        let result = deduplicate_modifiers(modifiers);
        assert_eq!(result, vec!["data", "public", "final"]);
    }

    #[test]
    fn test_parse_constructor_params() {
        let result = parse_parameters("(val name: String, var age: Int)");
        assert_eq!(result, vec!["val name: String", "var age: Int"]);
    }

    #[test]
    fn test_hover_signature_format() {
        let signature = HoverSignature::new("kotlin")
            .with_package(Some("com.example".to_string()))
            .with_annotations(vec!["@Component".to_string()])
            .with_modifiers(vec!["public".to_string()])
            .with_signature_line("class Example".to_string())
            .with_documentation(Some("This is documentation".to_string()));

        let formatted = signature.format();
        assert!(formatted.contains("com.example"));
        assert!(formatted.contains("@Component"));
        assert!(formatted.contains("public class Example"));
        assert!(formatted.contains("This is documentation"));
    }
    
    #[test]
    fn test_strip_comment_signifiers() {
        let docs = "/**\n * @author frank on 9/6/17.\n * This is a test\n */";
        let cleaned = strip_comment_signifiers(docs);
        assert_eq!(cleaned, "@author frank on 9/6/17.\nThis is a test");
    }
}