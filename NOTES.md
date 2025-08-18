# TODO
src/core/registry.rs:50: Implement cross-language symbol resolution
src/core/registry.rs:61: Implement targeted cross-language resolution
src/core/dependency_cache/symbol_index.rs:194: Implement Kotlin symbol extraction
src/core/definition/local.rs:73:                _ => &[], // TODO: Add more mappings as needed
src/core/definition/local.rs:240: Add signature-based method matching
src/core/definition/project.rs:16: Implement shared project search logic
src/core/definition/project.rs:34: Implement cross-language project search
src/core/definition/workspace.rs:16: Implement shared workspace search logic
src/core/definition/workspace.rs:34: Implement cross-language workspace search
src/core/definition/external.rs:16: Implement shared external dependency search logic
src/core/definition/external.rs:34: Implement cross-language external search
src/core/definition/resolution.rs:95: Implement cross-language reference detection
src/core/cross_language/imports.rs:19: Implement import extraction for different languages
src/core/cross_language/imports.rs:45: Implement import resolution
src/core/cross_language/imports.rs:56: Implement symbol resolution through imports
src/core/cross_language/type_bridge.rs:60: Implement Java -> Groovy type conversion
src/core/cross_language/type_bridge.rs:71: Implement Kotlin -> Java type conversion
src/core/cross_language/type_bridge.rs:82: Implement Groovy -> Java type conversion
src/core/cross_language/resolver.rs:19: Implement cross-language symbol resolution
src/core/cross_language/resolver.rs:30: Implement cross-language import resolution
src/core/cross_language/resolver.rs:41: Implement language detection from import paths
src/core/utils.rs:174: Implement Kotlin search_definition_in_project
src/languages/java/support.rs:373:            visibility: Visibility::Public, // TODO: extract actual visibility
src/languages/java/support.rs:374:            methods: vec![], // TODO: extract methods
src/languages/java/support.rs:375:            fields: vec![], // TODO: extract fields
src/languages/java/support.rs:392: Implement actual cross-language resolution logic
src/languages/java/definition/local.rs:171: Implement proper scope distance calculation
src/languages/java/definition/local.rs:255: Implement proper method overloading resolution based on parameter types
src/languages/java/implementation.rs:50: Implement actual implementation finding logic
src/languages/java/implementation.rs:64: Implement method call implementation finding
src/languages/java/implementation.rs:76: Implement method implementation finding
src/languages/kotlin/support.rs:117: Implement Kotlin parser setup when tree-sitter-kotlin is added
src/languages/kotlin/support.rs:127: Implement Kotlin-specific diagnostics
src/languages/kotlin/support.rs:139: Implement Kotlin-specific definition finding using shared algorithms
src/languages/kotlin/support.rs:152: Implement Kotlin implementation finding
src/languages/kotlin/support.rs:157: Implement Kotlin hover support
src/languages/kotlin/support.rs:167: Implement Kotlin-specific symbol type detection
src/languages/kotlin/support.rs:172: Implement Kotlin type info extraction for cross-language support
src/languages/kotlin/support.rs:183: Implement Kotlin cross-language definition finding
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
src/languages/groovy/definition/external.rs:29: currently accidentally work because the tree-sitter node names overlap

# HACK
src/languages/groovy/definition/workspace.rs:92: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:666: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:303: include everything else that's not explicitly skipped
