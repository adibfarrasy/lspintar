# TODO
src/languages/java/definition/local.rs:190: Implement proper scope distance calculation
src/languages/java/implementation.rs:122: Implement method call implementation finding
src/languages/java/implementation.rs:134: Implement method implementation finding
src/languages/kotlin/hover/mod.rs:322: Implement method declaration finding for method calls
src/languages/kotlin/implementation.rs:116: Implement method call -> implementation mapping
src/languages/kotlin/implementation.rs:129: Implement method implementation finding
src/languages/groovy/support.rs:146: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:564: Could enhance with variable type lookup
src/languages/groovy/implementation.rs:46: currently only handle interfaces.
src/languages/groovy/implementation.rs:171: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME

# HACK
src/languages/groovy/definition/workspace.rs:107: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:588: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:329: include everything else that's not explicitly skipped
