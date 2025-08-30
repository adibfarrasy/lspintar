# TODO
src/core/build_tools.rs:874: Could also check file size here if we want to be more sophisticated
src/languages/java/definition/local.rs:190: Implement proper scope distance calculation
src/languages/java/definition/utils.rs:508: Could verify this FQN exists in cache/database, but for now assume it's correct
src/languages/java/implementation.rs:122: Implement method call implementation finding
src/languages/java/implementation.rs:134: Implement method implementation finding
src/languages/groovy/support.rs:146: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:564: Could enhance with variable type lookup
src/languages/groovy/definition/utils.rs:1063: Could verify this FQN exists in cache/database, but for now assume it's correct
src/languages/groovy/implementation.rs:46: currently only handle interfaces.
src/languages/groovy/implementation.rs:171: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME

# HACK
src/languages/groovy/definition/workspace.rs:107: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/dependency_cache/builtin.rs:384: include everything else that's not explicitly skipped
