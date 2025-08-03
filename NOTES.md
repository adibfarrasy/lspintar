# TODO
src/core/dependency_cache/symbol_index.rs:179: Implement Java symbol extraction
src/core/dependency_cache/symbol_index.rs:183: Implement Kotlin symbol extraction
src/server.rs:342: implement parse_configuration
src/languages/groovy/support.rs:52: replace this with more sophisticated handling
src/languages/groovy/definition/local.rs:380: Could enhance with variable type lookup
src/languages/groovy/definition/mod.rs:6: make this private?
src/languages/groovy/definition/utils.rs:167: handle wildcard import
src/languages/groovy/implementation.rs:43: currently only handle interfaces.
src/languages/groovy/implementation.rs:159: currently using naive implementation
src/languages/groovy/symbols.rs:13: currently only handles non-nested declarations

# FIXME
src/core/build_tools.rs:394: this should be customizable by the user
src/languages/groovy/definition/external.rs:21: currently accidentally work because the tree-sitter node names overlap

# HACK

# WARN
src/core/logging_service.rs:30:            message_type: MessageType::WARNING,

# NOTE
src/core/build_tools.rs:489: use any reasonable number to get the first few lines
src/core/dependency_cache/builtin.rs:320: include everything else that's not explicitly skipped
src/languages/groovy/definition/workspace.rs:90: Naive implementation, does not consider whether dependency is valid,
