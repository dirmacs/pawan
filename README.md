<p align="center">
  <img src="docs/img/pawan-logo.svg" alt="Pawan" width="280">
</p>

<p align="center">
  <strong>Self-healing, self-improving CLI coding agent</strong>
</p>

<p align="center">
  <a href="https://github.com/dirmacs/pawan/actions"><img src="https://github.com/dirmacs/pawan/workflows/CI/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/pawan-core"><img src="https://img.shields.io/crates/v/pawan-core.svg" alt="crates.io"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
</p>

---

**Pawan** (पवन, "wind" in Sanskrit) is a Rust-native CLI coding agent that reads, writes, and heals code autonomously. Built for the [dirmacs](https://github.com/dirmacs) ecosystem.

## Features

- **Tool-calling agent loop** — reads files, edits code, runs bash, searches, and commits
- **Self-healing** — automatically fixes compilation errors, clippy warnings, and test failures
- **Multi-provider LLM** — NVIDIA NIM, Ollama, OpenAI-compatible APIs
- **Interactive TUI** — ratatui-based terminal interface with streaming responses
- **Headless mode** — scriptable single-prompt execution for CI and orchestration
- **11 built-in tools** — bash, read/write/edit files, glob/grep search, git operations

## Quick Start

```bash
# Install from source
cargo install --path crates/pawan-cli

# Set your NVIDIA API key
export NVIDIA_API_KEY=nvapi-...

# Interactive mode
pawan

# Self-heal current project
pawan heal

# Execute a coding task
pawan task "add input validation to the config parser"

# Generate a commit message
pawan commit

# Check project health
pawan status
```

## Architecture

```
pawan/
  crates/
    pawan-core/    # Library: agent engine, tools, config, healing
    pawan-cli/     # Binary: CLI interface + ratatui TUI
```

**pawan-core** has zero dirmacs-internal dependencies — it works standalone with any OpenAI-compatible API. Future crates (`pawan-mcp`, `pawan-aegis`) add MCP server support and [aegis](https://github.com/dirmacs/aegis) config integration.

## Configuration

Create `pawan.toml` in your project root:

```toml
provider = "nvidia"
model = "mistralai/devstral-2-123b-instruct-2512"
temperature = 0.6
max_tool_iterations = 50

[healing]
fix_errors = true
fix_warnings = true
fix_tests = true
auto_commit = false

[tui]
syntax_highlighting = true
mouse_support = true
```

## Subcommands

| Command | Description |
|---------|-------------|
| `pawan` | Interactive TUI chat (default) |
| `pawan chat` | Same as above |
| `pawan heal` | Auto-fix errors, warnings, and failing tests |
| `pawan task "..."` | Execute a specific coding task |
| `pawan commit` | Generate and apply a commit message |
| `pawan improve docs\|refactor\|tests\|all` | Improve code quality |
| `pawan status` | Show project health summary |

## Tools

Pawan ships with 11 tools available to the LLM:

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with timeout |
| `read_file` | Read file contents |
| `write_file` | Write entire files |
| `edit_file` | Precise string replacement |
| `list_directory` | List directory contents |
| `glob_search` | Find files by pattern |
| `grep_search` | Search file contents |
| `git_status` | Check repository status |
| `git_diff` | Show changes |
| `git_add` | Stage files |
| `git_commit` | Create commits |

## Dirmacs Ecosystem

Pawan is the foundational coding agent in the dirmacs toolchain:

- **[aegis](https://github.com/dirmacs/aegis)** — Declarative config management
- **[ares](https://github.com/dirmacs/ares)** — Agentic retrieval-enhanced server
- **[daedra](https://github.com/dirmacs/daedra)** — Web search MCP server
- **[nimakai](https://github.com/dirmacs/nimakai)** — NIM model latency benchmarker

## License

MIT
