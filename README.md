# lspintar

[![CI](https://github.com/adibfarrasy/lspintar/actions/workflows/ci.yml/badge.svg)](https://github.com/adibfarrasy/lspintar/actions/workflows/ci.yml)

A Language Server Protocol (LSP) server for Java, Groovy, and Kotlin — built to be fast, lightweight, and free.

## Why

IntelliJ is the de facto standard for JVM development, but it is expensive, resource-heavy, and locked to its own editor. Open source alternatives like Eclipse JDT LS either require a running JVM process, carry significant memory overhead, or offer incomplete support for the full JVM language family — particularly Groovy.

lspintar is built differently. It indexes your workspace into a local SQLite database and answers LSP queries from that index, with no JVM process involved. It uses a fraction of the memory of IntelliJ — in practice, often 99% less — at the cost of some disk space for the index. The goal is to bring first-class Java, Groovy, and Kotlin navigation to any LSP-capable editor, for free.

**Status: alpha.** Core navigation, refactoring, and completion features work. A growing set of diagnostics is implemented (see below); full type-checker parity with IntelliJ is not a goal.

## Features

- **Go to definition** — workspace source files and external JAR dependencies
- **Go to implementation** — interfaces, abstract methods, and overridden methods
- **Find references** — cross-file and cross-language (Java ↔ Groovy ↔ Kotlin)
- **Rename** — signature-matched hierarchy walk, scope-aware for locals, parameters, and closure/lambda bindings; rejects invalid identifiers and reserved keywords
- **Hover** — classes, methods, fields, interfaces; markdown is tagged with the producer's source language
- **Completion** — chained member access, prefix completion, local-before-global ranking, implicit-import resolution (Groovy `groovy.lang.*`, etc.)
- **Diagnostics** — `unimplemented_abstract_methods` (signature-aware, overload-aware), `final_class_extended` (incl. Kotlin's final-by-default), `syntax_error`, `unresolved_symbol`, `method_not_found`, `wrong_argument_types`, `narrowing_conversion`, and more
- **Cross-language interop** — a Groovy file can import Java and Kotlin classes (and vice versa) with all of the above features working across the boundary
- **Dependency indexing** — reads JAR files from the Gradle cache; decompiles bytecode when source is unavailable
- **Incremental re-indexing** on file save and on VCS revision change between startups

## Prerequisites

- Rust toolchain (`cargo`)
- `just` task runner (`cargo install just`)

## Building from source

```bash
git clone https://github.com/adibfarrasy/lspintar
cd lspintar
just b
```

The binary is at `target/release/lspintar`.

## Installation

### Neovim

Register lspintar as a custom server with `nvim-lspconfig`:

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')
local util = require('lspconfig.util')

if not configs.lspintar then
  configs.lspintar = {
    default_config = {
      cmd = { '/path/to/lspintar' },
      filetypes = { 'java', 'groovy', 'kotlin' },
      root_dir = function(fname)
        return util.root_pattern('settings.gradle', 'settings.gradle.kts', 'pom.xml', '.git')(fname)
      end,
      init_options = {
        gradle_cache_dir = os.getenv('HOME') .. '/.gradle/caches/modules-2/files-2.1',
      },
    },
  }
end

lspconfig.lspintar.setup {}
```

Replace `/path/to/lspintar` with the path to the binary you built.

### VS Code and Cursor

The [`lspintar-vscode`](https://github.com/adibfarrasy/lspintar-vscode) extension connects VS Code (and Cursor, which uses the same extension API) to a locally built lspintar binary.

1. Clone and build the extension:

```bash
git clone https://github.com/adibfarrasy/lspintar-vscode
cd lspintar-vscode
npm install
npm run compile
```

2. Install as a development extension by symlinking the folder into your editor's extensions directory:

```bash
# Cursor
ln -s "$(pwd)" ~/.cursor/extensions/lspintar-vscode

# VS Code
ln -s "$(pwd)" ~/.vscode/extensions/lspintar-vscode
```

3. Register the extension in `extensions.json` so the editor loads it. Edit `~/.cursor/extensions/extensions.json` (or `~/.vscode/extensions/extensions.json`) and add this entry to the JSON array (adjust the paths if your home directory differs):

```json
{
  "identifier": { "id": "undefined_publisher.lspintar" },
  "version": "0.0.1",
  "location": {
    "$mid": 1,
    "fsPath": "/Users/you/.cursor/extensions/lspintar-vscode",
    "external": "file:///Users/you/.cursor/extensions/lspintar-vscode",
    "path": "/Users/you/.cursor/extensions/lspintar-vscode",
    "scheme": "file"
  },
  "relativeLocation": "lspintar-vscode"
}
```

If the file doesn't exist, create it as `[ { ... } ]`. Fully quit and reopen the editor (`Cmd+Q` on macOS — closing the window is not enough).

4. Set the server path in your settings:

```json
{
  "lspintar.serverPath": "/path/to/lspintar"
}
```

## Configuration

| Option | Description | Default |
|--------|-------------|---------|
| `gradle_cache_dir` | Path to the Gradle files cache | — |
| `build_on_init` | Trigger a Gradle build when the server starts | `false` |

## Development

```bash
# Build
just b

# Run all tests (includes integration tests)
just tt

# Run tests for a specific package
just tp lsp_core
```

Integration tests are gated behind the `integration-test` feature flag and run against the Gradle fixtures under `server/tests/fixtures/`. Each test binary copies its fixture into a per-process tempdir, so the suite is safe to run in parallel — no `--test-threads=1` required. On first run, the Gradle handler downloads classpath and source jars into `~/.gradle/caches`; subsequent runs are fast.

CI runs the full parallel suite on every push and PR — see `.github/workflows/ci.yml`.

## License

MIT
