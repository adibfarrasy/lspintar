# TODO
src/core/registry.rs:50: Implement cross-language symbol resolution
src/core/registry.rs:61: Implement targeted cross-language resolution
src/core/dependency_cache/symbol_index.rs:194: Implement Kotlin symbol extraction
src/core/definition/local.rs:73:                _ => &[], // TODO: Add more mappings as needed
src/core/definition/local.rs:240: Add signature-based method matching
src/core/utils.rs:177: Implement Kotlin search_definition_in_project
src/languages/java/definition/local.rs:193: Implement proper scope distance calculation
src/languages/java/implementation.rs:50: Implement actual implementation finding logic
src/languages/java/implementation.rs:64: Implement method call implementation finding
src/languages/java/implementation.rs:76: Implement method implementation finding
src/languages/kotlin/support.rs:101: Implement Kotlin parser setup when tree-sitter-kotlin is added
src/languages/kotlin/support.rs:111: Implement Kotlin-specific diagnostics
src/languages/kotlin/support.rs:123: Implement Kotlin-specific definition finding using shared algorithms
src/languages/kotlin/support.rs:136: Implement Kotlin implementation finding
src/languages/kotlin/support.rs:141: Implement Kotlin hover support
src/languages/kotlin/support.rs:151: Implement Kotlin-specific symbol type detection
src/languages/kotlin/support.rs:173: Implement Kotlin project definition finding
src/languages/kotlin/support.rs:184: Implement Kotlin workspace definition finding
src/languages/kotlin/support.rs:195: Implement Kotlin external definition finding
src/languages/kotlin/support.rs:205: Implement Kotlin-specific position setting
src/languages/groovy/support.rs:146: replace this with more sophisticated handling
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
src/core/dependency_cache/builtin.rs:322: include everything else that's not explicitly skipped
