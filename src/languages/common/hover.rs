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
    pub additional_info: Vec<String>,
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
            additional_info: Vec::new(),
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

    pub fn with_constructor_params(mut self, params: Vec<String>) -> Self {
        self.constructor_params = params;
        self
    }

    pub fn with_documentation(mut self, docs: Option<String>) -> Self {
        self.documentation = docs;
        self
    }

    pub fn add_info(mut self, info: String) -> Self {
        self.additional_info.push(info);
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
                parts.push("\n".to_string());
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

        // Constructor parameters - handle Kotlin special formatting
        if !self.constructor_params.is_empty() {
            // Check if this is a data class (signature contains "data class")
            let is_data_class = self.signature_line.contains("data class") || 
                               self.signature_line.contains("data object");
            
            // For Kotlin, split constructor args into multiple lines if more than 3 args or is data class
            let should_split_args = self.language == "kotlin" && 
                                   (is_data_class || self.constructor_params.len() > 3);
            
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
                // Regular classes: parameters inline
                let params_str = self.constructor_params.join(", ");
                parts.push(format!("({})", params_str));
            }
        }

        // Add inheritance line - only prefix with ':' for Kotlin
        if let Some(ref inheritance) = self.inheritance {
            if !inheritance.is_empty() {
                let inheritance_line = if self.language == "kotlin" {
                    if inheritance.starts_with(':') {
                        inheritance.clone()
                    } else {
                        format!(": {}", inheritance)
                    }
                } else {
                    // For Java/Groovy, just use inheritance as-is
                    inheritance.clone()
                };
                parts.push(inheritance_line);
            }
        }

        // End code block
        parts.push("```".to_string());

        // Add additional info
        for info in &self.additional_info {
            if !info.is_empty() {
                parts.push(format!("\n{}", info));
            }
        }

        // Documentation with separator and stripped comment signifiers
        if let Some(ref docs) = self.documentation {
            if !docs.is_empty() {
                parts.push("\n".to_string());
                parts.push("---".to_string());
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

/// Parse constructor parameters from raw text into individual parameter lines
pub fn parse_constructor_params(constructor_text: &str) -> Vec<String> {
    if constructor_text.is_empty() {
        return Vec::new();
    }

    let content = constructor_text
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