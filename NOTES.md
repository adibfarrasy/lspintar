# TODO
src/core/dependency_cache/symbol_index.rs:193: Implement Kotlin symbol extraction
src/core/utils.rs:172: Implement Kotlin search_definition_in_project
src/languages/java/definition/local.rs:190: Implement proper scope distance calculation
src/languages/java/implementation.rs:122: Implement method call implementation finding
src/languages/java/implementation.rs:134: Implement method implementation finding
src/languages/kotlin/hover/interface.rs:2: Implement hover for Kotlin interface declarations
src/languages/kotlin/hover/field.rs:2: Implement hover for Kotlin property declarations
src/languages/kotlin/hover/mod.rs:17: Implement Kotlin hover support
src/languages/kotlin/hover/class.rs:2: Implement hover for Kotlin class declarations
src/languages/kotlin/hover/method.rs:2: Implement hover for Kotlin function declarations
src/languages/kotlin/hover/utils.rs:2: Implement hover utilities for Kotlin symbols
src/languages/kotlin/definition/project.rs:13: Implement Kotlin project-wide definition search
src/languages/kotlin/definition/workspace.rs:13: Implement Kotlin workspace-wide definition search
src/languages/kotlin/definition/external.rs:12: Implement Kotlin external definition search
src/languages/kotlin/definition/method_resolution.rs:4: Implement Kotlin method signature extraction
src/languages/kotlin/implementation.rs:14: Implement Kotlin implementation finding
src/languages/kotlin/symbols.rs:13: Implement symbol collection when tree_sitter_kotlin is available
src/languages/groovy/support.rs:146: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:554: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:7: make this private?
src/languages/groovy/implementation.rs:46: currently only handle interfaces.
src/languages/groovy/implementation.rs:171: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME

# HACK
src/languages/groovy/definition/workspace.rs:107: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:585: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:329: include everything else that's not explicitly skipped
