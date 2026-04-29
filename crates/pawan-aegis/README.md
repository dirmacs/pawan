# pawan-aegis

Pawan-Aegis integration — generate `pawan.toml` configuration from Aegis manifests.

## Overview

This crate provides integration between Pawan and the [Aegis](https://github.com/dirmacs/aegis) configuration management system. It reads `[pawan]` sections from Aegis manifests and generates properly formatted `pawan.toml` configuration files.

## Features

- **Aegis Manifest Parsing** — Reads `[pawan]` sections from Aegis TOML manifests
- **Configuration Generation** — Generates `pawan.toml` with proper structure
- **Provider Support** — Handles multiple providers (nvidia, ollama, openai, mlx)
- **MCP Server Configuration** — Configures MCP servers with commands, args, and environment variables
- **Healing Configuration** — Supports auto-healing settings (fix_errors, fix_warnings, fix_tests, auto_commit)
- **Smart Defaults** — Uses sensible defaults and omits unnecessary fields

## Installation

Add the library from crates.io:

```bash
cargo add pawan-aegis
```

## Usage

### Basic Usage

```rust
use pawan_aegis::PawanInput;

// Load pawan configuration from an aegis manifest
let input = PawanInput::load("path/to/aegis-manifest.toml")?
    .expect("pawan section not found");

// Generate pawan.toml content
let toml_content = input.generate()?;

// Write to file
input.write_to("pawan.toml")?;
```

### Aegis Manifest Example

```toml
# aegis-manifest.toml
[pawan]
provider = "nvidia"
model = "qwen/qwen3.5-122b-a10b"
temperature = 0.6
max_tokens = 4096

[pawan.mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
enabled = true

[pawan.healing]
fix_errors = true
fix_warnings = false
fix_tests = true
auto_commit = false
```

### Generated pawan.toml

```toml
# Generated pawan.toml
model = "qwen/qwen3.5-122b-a10b"
temperature = 0.6
max_tokens = 4096

[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
enabled = true

[healing]
fix_errors = true
fix_warnings = false
fix_tests = true
auto_commit = false
```

## Configuration Fields

### Main Configuration

- `provider` — LLM provider (nvidia, ollama, openai, mlx)
- `model` — Model identifier or key
- `temperature` — Sampling temperature (0.0-2.0)
- `top_p` — Nucleus sampling parameter
- `max_tokens` — Maximum output tokens

### MCP Servers

- `command` — MCP server executable
- `args` — Command-line arguments
- `env` — Environment variables
- `enabled` — Whether the server is active

### Healing Configuration

- `fix_errors` — Auto-fix compilation errors
- `fix_warnings` — Auto-fix clippy warnings
- `fix_tests` — Auto-fix failing tests
- `auto_commit` — Auto-commit fixes

## Smart Defaults

- **Provider**: Defaults to `nvidia` (omitted from generated TOML if default)
- **Model**: Falls back to `qwen/qwen3.5-122b-a10b` if not specified
- **MCP enabled**: Defaults to `true`
- **Empty sections**: Omitted from generated TOML

## Integration with Pawan

This crate is used internally by Pawan to integrate with Aegis-based configuration management. It provides a clean interface for reading Aegis manifests and generating Pawan configuration files.

The crate provides comprehensive error handling via the `AegisError` enum:

```rust
use pawan_aegis::{AegisError, Result};

fn load_config() -> Result<String> {
    let input = PawanInput::load("manifest.toml")?
        .ok_or_else(|| AegisError::Config("No pawan section found".to_string()))?;
    input.generate()
}
```

## Testing

The crate includes comprehensive tests covering:

- Manifest parsing
- TOML generation
- Default value handling
- Provider-specific behavior
- Round-trip conversion
- Healing configuration

Run tests with:
```bash
cargo test --package pawan-aegis
```

## License

MIT

## See Also

- [Pawan](https://github.com/dirmacs/pawan) — Main CLI coding agent
- [Aegis](https://github.com/dirmacs/aegis) — Configuration management system