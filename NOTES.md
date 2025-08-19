# TODO
src/core/registry.rs:50: Implement cross-language symbol resolution
src/core/registry.rs:61: Implement targeted cross-language resolution
src/core/dependency_cache/symbol_index.rs:194: Implement Kotlin symbol extraction
src/core/definition/local.rs:73:                _ => &[], // TODO: Add more mappings as needed
src/core/definition/local.rs:240: Add signature-based method matching
src/core/utils.rs:177: Implement Kotlin search_definition_in_project
src/languages/java/support.rs:373:            visibility: Visibility::Public, // TODO: extract actual visibility
src/languages/java/support.rs:374:            methods: vec![], // TODO: extract methods
src/languages/java/support.rs:375:            fields: vec![], // TODO: extract fields
src/languages/java/support.rs:392: Implement actual cross-language resolution logic
src/languages/java/definition/local.rs:170: Implement proper scope distance calculation
src/languages/java/definition/local.rs:254: Implement proper method overloading resolution based on parameter types
src/languages/java/implementation.rs:50: Implement actual implementation finding logic
src/languages/java/implementation.rs:64: Implement method call implementation finding
src/languages/java/implementation.rs:76: Implement method implementation finding
src/languages/kotlin/support.rs:114: Implement Kotlin parser setup when tree-sitter-kotlin is added
src/languages/kotlin/support.rs:124: Implement Kotlin-specific diagnostics
src/languages/kotlin/support.rs:136: Implement Kotlin-specific definition finding using shared algorithms
src/languages/kotlin/support.rs:149: Implement Kotlin implementation finding
src/languages/kotlin/support.rs:154: Implement Kotlin hover support
src/languages/kotlin/support.rs:164: Implement Kotlin-specific symbol type detection
src/languages/kotlin/support.rs:169: Implement Kotlin type info extraction for cross-language support
src/languages/kotlin/support.rs:180: Implement Kotlin cross-language definition finding
src/languages/kotlin/support.rs:202: Implement Kotlin project definition finding
src/languages/kotlin/support.rs:213: Implement Kotlin workspace definition finding
src/languages/kotlin/support.rs:224: Implement Kotlin external definition finding
src/languages/kotlin/support.rs:234: Implement Kotlin-specific position setting
src/languages/groovy/support.rs:146: replace this with more sophisticated handling
src/languages/groovy/support.rs:498: Implement type info extraction for Groovy
src/languages/groovy/support.rs:509: Implement cross-language definition finding for Groovy
src/languages/groovy/definition/local.rs:554: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:7: make this private?
src/languages/groovy/implementation.rs:46: currently only handle interfaces.
src/languages/groovy/implementation.rs:162: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME

# HACK
src/languages/groovy/definition/workspace.rs:91: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:653: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:303: include everything else that's not explicitly skipped
