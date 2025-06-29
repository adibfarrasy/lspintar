# TODO
src/core/dependency_cache/mod.rs:73: implement
src/core/dependency_cache/symbol_index.rs:170: Implement Java symbol extraction
src/core/dependency_cache/symbol_index.rs:174: Implement Kotlin symbol extraction
src/core/dependency_cache/builtin.rs:33: must detect build tool first, then handle accordingly.
src/languages/groovy/hover/interface.rs:91: Add docstring extraction
src/languages/groovy/hover/class.rs:106: Add docstring extraction
src/languages/groovy/support.rs:46: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:376: Could enhance with variable type lookup
src/languages/groovy/definition/utils.rs:291: handle wildcard import
src/languages/groovy/implementation.rs:42: currently only handle interfaces.
src/languages/groovy/implementation.rs:61: because it's always looping this has performance issue
src/languages/groovy/implementation.rs:194: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME
src/core/dependency_cache/builtin.rs:271: integrate with build tool to resolve imports
src/languages/groovy/definition/external.rs:16: implement

# HACK

# WARN

# NOTE
src/languages/groovy/definition/workspace.rs:20: Naive implementation, does not consider whether dependency is
