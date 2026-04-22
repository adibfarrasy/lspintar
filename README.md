# lspintar

A Language Server Protocol (LSP) server for Java, Groovy, and Kotlin — built to be fast, lightweight, and free.

## Why

IntelliJ is the de facto standard for JVM development, but it is expensive, resource-heavy, and locked to its own editor. Open source alternatives like Eclipse JDT LS either require a running JVM process, carry significant memory overhead, or offer incomplete support for the full JVM language family — particularly Groovy.

lspintar is built differently. It indexes your workspace into a local SQLite database and answers LSP queries from that index, with no JVM process involved. It uses a fraction of the memory of IntelliJ — in practice, often 99% less — at the cost of some disk space for the index. The goal is to bring first-class Java, Groovy, and Kotlin navigation to any LSP-capable editor, for free.

**Status: alpha.** Core navigation features work. Diagnostics are not yet implemented.

## Features

- Go to definition — workspace source files and external JAR dependencies
- Go to implementation — interfaces and abstract methods
- Hover information — classes, methods, fields, interfaces
- Dependency indexing — reads JAR files from the Gradle cache; decompiles bytecode when source is unavailable
- Incremental re-indexing on build file changes

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

Integration tests require a real Gradle project on disk; they are gated behind the `integration-test` feature flag and run with `--test-threads=1`.

## License

MIT
