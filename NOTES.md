# TODO
src/core/dependency_cache/symbol_index.rs:179: Implement Java symbol extraction
src/core/dependency_cache/symbol_index.rs:183: Implement Kotlin symbol extraction
src/core/dependency_cache/builtin.rs:47: currently assumes it's a groovy project. should check if the imports are necessary.
src/languages/groovy/hover/interface.rs:91: Add docstring extraction
src/languages/groovy/hover/class.rs:106: Add docstring extraction
src/languages/groovy/support.rs:53: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:380: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:7: make this private?
src/languages/groovy/definition/utils.rs:167: handle wildcard import
src/languages/groovy/implementation.rs:43: currently only handle interfaces.
src/languages/groovy/implementation.rs:159: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME
src/core/dependency_cache/builtin.rs:278: integrate with build tool to resolve imports
src/languages/groovy/definition/builtin.rs:16: currently accidentally work because the tree-sitter node names overlap
src/languages/groovy/definition/external.rs:16: implement

# HACK

# WARN

# NOTE
src/languages/groovy/definition/workspace.rs:24: Naive implementation, does not consider whether dependency is
