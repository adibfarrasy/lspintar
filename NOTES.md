# TODO
src/core/dependency_cache/symbol_index.rs:192: Implement Java symbol extraction
src/core/dependency_cache/symbol_index.rs:196: Implement Kotlin symbol extraction
src/languages/groovy/support.rs:55: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:408: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:7: make this private?
src/languages/groovy/implementation.rs:46: currently only handle interfaces.
src/languages/groovy/implementation.rs:162: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME
src/languages/groovy/definition/external.rs:29: currently accidentally work because the tree-sitter node names overlap

# HACK
src/languages/groovy/definition/workspace.rs:91: Naive implementation, does not consider whether dependency is valid,

# WARN

# NOTE
src/core/build_tools.rs:553: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:303: include everything else that's not explicitly skipped
