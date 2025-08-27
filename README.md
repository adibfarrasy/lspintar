# LSPintar

A Language Server Protocol (LSP) implementation for JVM-based languages, supporting:
- <span style="font-size: 2em;"><img src="https://github.com/devicons/devicon/blob/master/icons/java/java-original.svg" height="32" style="vertical-align: text-bottom"> Java </span>
- <span style="font-size: 2em;"><img src="https://github.com/devicons/devicon/blob/master/icons/groovy/groovy-original.svg" height="32" style="vertical-align: text-bottom"> Groovy</span>
- <span style="font-size: 2em;"><img src="https://github.com/devicons/devicon/blob/master/icons/kotlin/kotlin-original.svg" height="32" style="vertical-align: text-bottom"> Kotlin</span>

## Features

### Current Support
- **Go to Definition**: Navigate to symbol definitions across local files, project files, workspace, and external dependencies
- **Go to Implementation**: Find implementations of interfaces and abstract methods
- **Hover Information**: Display detailed information about classes, methods, fields, and interfaces
- **Diagnostics**: Real-time syntax error detection and reporting
- **Symbol Resolution**: Intelligent symbol lookup with context-aware type determination
- **Dependency Cache**: Efficient caching system for external dependency resolution
- **Java Bytecode Decompilation**: Automatic decompilation of .class files when source files are unavailable

### Planned Support
Additional features under development:
- Code completion
- Workspace symbols
- Document symbols
- Code formatting
- Refactoring capabilities

## Installation

### Neovim with lspconfig

#### Basic Setup

```lua
require('lspconfig').lspintar.setup {}
```

#### Complete Configuration

```lua
local util = require 'lspconfig.util'
local configs = require 'lspconfig.configs'

local lspintar_path = '/path/to/your/lspintar'

if not configs.lspintar then
  configs.lspintar = {
    default_config = {
      cmd = { lspintar_path },
      filetypes = { 'groovy' },
      root_dir = function(fname)
        return util.root_pattern('settings.gradle', '.git')(fname)
          or util.root_pattern('build.gradle', 'pom.xml')(fname)
          or util.root_pattern 'META-INF'(fname)
      end,
      init_options = {
        gradle_cache_dir = os.getenv 'HOME' .. '/.sdkman/candidates/gradle/6.3/caches/modules-2/files-2.1',
        build_on_init = false,
      },
      settings = {},
    },
  }
end

require('lspconfig').lspintar.setup {}
```

## Building from Source

```bash
cargo build --release
```

The binary will be available at `target/release/lspintar`.

## Decompiler Setup

LSPintar includes Java bytecode decompilation capabilities to provide "go to definition" functionality for dependencies that don't have source files available (such as Spring Framework libraries).

### Automatic Setup

The decompiler will automatically download and set up the CFR decompiler on first use. No manual configuration is required.

### Manual Setup (Optional)

If you prefer to set up the decompiler manually:

1. Download the CFR decompiler JAR from [CFR Releases](https://github.com/leibnitz27/cfr/releases)
2. Place it in `~/.local/share/lspintar/cfr-0.152.jar`

### How it Works

- When navigating to symbols in external dependencies, LSPintar first tries to find source files
- If source files are unavailable (e.g., only .class files in JAR), it automatically decompiles the bytecode
- Decompiled content is cached and indexed for fast symbol resolution
- Works with all JVM languages: Java, Kotlin, Groovy classes are all decompiled to Java source

## Configuration Options

- `gradle_cache_dir`: Path to Gradle cache directory for dependency resolution
- `build_on_init`: Whether to build the project on initialization (default: false)

## License

This project is licensed under the MIT License.
