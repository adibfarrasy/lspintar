# Scala Language Support — Design

**Status:** in-progress
**Owner:** Adib
**Date:** 2026-05-17
**Branch:** `feat/scala-support`

## Goal

Add Scala as a first-class language in lspintar with feature parity to the existing Java / Kotlin / Groovy crates, including JVM-level interop (Scala code resolving Java/Kotlin/Groovy symbols and vice versa).

## Non-Goals

- Scala-native build tool integration (sbt/mill). Files are still indexed under whatever Gradle/Maven setup the user has; sbt project layout integration is deferred.
- Macro expansion, inline def expansion, or givens-resolution at type-checking depth. The LSP treats macros and givens as opaque method calls.
- Full HKT / path-dependent type resolution. Best-effort short-name matching matches the existing Kotlin/Groovy behaviour.

## Scope of Dialects

Target **both Scala 2.13 and Scala 3** behind a single crate using the upstream `tree-sitter-scala` grammar (which parses both). Build the dialect-shared core first (classes, objects, traits, defs, vals, imports, packages); add 3-only nodes (`given_definition`, `extension_definition`, `enum_definition`, optional-braces blocks) once the shared core is green.

## Grammar Dependency

- **Initial:** `tree-sitter-scala = "0.26"` from crates.io (upstream `tree-sitter/tree-sitter-scala`).
- **Final:** swap to `tree-sitter-scala = { git = "https://github.com/adibfarrasy/tree-sitter-scala" }` once the fork exists, matching the pattern used for java/kotlin/groovy.

The swap is a one-line `Cargo.toml` change; no Rust code depends on the dependency source.

## Architecture

Mirror the existing language crate layout:

```
scala/
  Cargo.toml
  src/
    lib.rs                 # mod constants; mod support; pub use ScalaSupport
    constants.rs           # SCALA_IMPLICIT_IMPORTS
    support/
      mod.rs               # impl LanguageSupport for ScalaSupport
      queries.rs           # tree-sitter queries
      tests/               # integration tests against fixture files
```

`Backend::new` (server/src/server.rs) registers `ScalaSupport` under the `"scala"` extension. The `Language` enum (lsp_core/src/languages.rs) gets a `Scala` variant. `NodeKind::keyword` gets `"scala"` branches.

## Trait Surface

All ~30 methods of `LanguageSupport`. Phased delivery:

### Phase 1 — Scaffold (compiles + indexes)
- `get_language`, `get_ts_language`, `parse`, `parse_str`
- `get_range`, `get_ident_range`
- `get_kind`, `get_short_name`, `get_package_name`
- `get_imports`, `get_implicit_imports`, `should_index`
- `collect_diagnostics` (syntax errors only)
- **Tests:** parse a `.scala` file with package + class + def; assert symbols are produced.

### Phase 2 — Hierarchy + metadata
- `get_extends`, `get_implements` (Scala uses `extends A with B with C`)
- `get_modifiers`, `get_annotations`, `get_documentation`
- `get_parameters`, `get_return`
- **Tests:** hierarchy fixtures with traits and case classes; assert extends/with linearization is captured.

### Phase 3 — Position + type resolution
- `find_ident_at_position`, `get_type_at_position`
- `find_variable_type`, `find_variable_declaration`
- `find_declarations_in_scope`, `extract_call_arguments`
- `get_literal_type`, `get_method_receiver_and_params`
- `find_local_references`
- **Tests:** go-to-definition / find-references fixtures.

### Phase 4 — Diagnostics data
- `get_type_references`, `get_declared_type_names`
- `get_class_declarations`, `get_object_creations`, `get_member_accesses`
- `get_generic_type_usages`, `get_override_methods`, `get_method_call_sites`
- (No `get_narrowing_candidates` — Scala outlaws implicit numeric narrowing like Kotlin)
- **Tests:** unresolved-symbol, abstract-method, override-mismatch fixtures.

### Phase 5 — Scala 3-specific nodes
- `given_definition` → treat as `Field` / `Function` for indexing
- `extension_definition` → each method inside becomes a `Function` whose receiver is the extension target
- `enum_definition` → `NodeKind::Enum`; cases become `Field` entries with the enum as parent
- Optional braces — handled by grammar; no special trait handling needed
- **Tests:** Scala 3 syntax fixtures.

### Phase 6 — JVM interop
- `normalize_param_type` already canonicalises `Int`/`Integer`/`int`. Verify Scala types flow through.
- Add Scala-side type-name canonicalisation for `Unit` ↔ `void`, `AnyRef` ↔ `Object`, `AnyVal`, primitive boxing.
- Cross-language fixture: polyglot project with `.java`, `.kt`, `.groovy`, `.scala` files referencing each other's classes. Assert go-to-definition resolves across boundaries.

## Implicit Imports

Scala's `Predef`, `scala`, `scala.Predef._`, `java.lang.*`. Initial set:

```rust
pub const SCALA_IMPLICIT_IMPORTS: &[&str] = &[
    "scala.*",
    "scala.Predef.*",
    "scala.collection.immutable.*",
    "java.lang.*",
];
```

## Tree-sitter Node Kinds (key mappings)

| `NodeKind` | Scala 2 nodes | Scala 3 nodes |
|---|---|---|
| Class | `class_definition`, `case_class_definition` | same + `enum_definition` cases (sealed) |
| Interface | `trait_definition` | same |
| Function | `function_definition`, `function_declaration` | same + extension methods |
| Field | `val_definition`, `var_definition`, `val_declaration`, `var_declaration` | same + `given_definition` |
| Enum | (sealed traits + case objects, best-effort) | `enum_definition` |
| Annotation | annotation site `(annotation)` — no declaration form in Scala (annotations are classes extending `Annotation`) | same |

## Risks

1. **Scala 3 optional braces** — indentation-significant. tree-sitter-scala handles it, but our scope-traversal code (mirrored from Kotlin) assumes brace-delimited blocks. We may need a node-kind agnostic "block boundary" helper.
2. **`with` linearization for override matching** — Scala MRO differs from Java/Kotlin. Phase 4 ships best-effort: we treat each `with` as a direct parent and rely on the existing `MethodSig::implements` matcher. Edge cases (diamond inheritance, trait stacking with `super[T].method`) deferred.
3. **`given`/`using` resolution** — out of scope. Implicit parameters/values appear as regular params/vals for indexing; resolution is opaque.
4. **Macros** — same as givens: opaque method calls.

## Open Questions (deferred decisions)

- Should `case class` constructors generate a synthetic `apply` in the `object` companion for go-to-definition? *(Phase 4 decision; default = no, ship best-effort.)*
- Should `enum`'s cases be indexed as `Field` (Kotlin-style) or as `Class` (Java enum-style)? *(Phase 5 decision; lean Field.)*

## Definition of Done

1. `just b` passes clean.
2. All existing tests still pass.
3. Each phase's listed tests pass.
4. Polyglot fixture (Phase 6) demonstrates cross-language go-to-definition.
5. `Cargo.toml` swapped to `adibfarrasy/tree-sitter-scala` fork.
