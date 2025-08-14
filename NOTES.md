# TODO
src/core/dependency_cache/symbol_index.rs:192: Implement Java symbol extraction
src/core/dependency_cache/symbol_index.rs:196: Implement Kotlin symbol extraction
src/languages/groovy/support.rs:52: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:381: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:6: make this private?
src/languages/groovy/implementation.rs:43: currently only handle interfaces.
src/languages/groovy/implementation.rs:159: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME
src/languages/groovy/definition/external.rs:28: currently accidentally work because the tree-sitter node names overlap

# HACK
src/languages/groovy/definition/workspace.rs:90: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:493: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:303: include everything else that's not explicitly skipped
