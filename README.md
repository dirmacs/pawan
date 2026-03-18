# Pawan — Self-healing CLI Coding Agent

<p align="center">
  <strong>पवन — Your AI-powered development partner</strong>
</p>

<p align="center">
  <a href="https://github.com/dirmacs/pawan/actions"><img src="https://github.com/dirmacs/pawan/workflows/CI/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/tools-17-green.svg" alt="17 tools">
  <img src="https://img.shields.io/badge/tests-107-brightgreen.svg" alt="107 tests">
</p>

---

**Pawan** (पवन) is a Rust-native CLI coding agent that reads, writes, and heals code autonomously. No subscription, no vendor lock-in. Works with any OpenAI-compatible API including NVIDIA NIM, local Ollama, or llama.cpp via localhost.

## Why Pawan

- **Self-hosted** — runs on your machine, uses your own API keys
- **17 tools** — file ops, search, bash, git (status/diff/add/commit/log/blame/branch/checkout/stash), sub-agents
- **18 subcommands** — from interactive TUI to headless scripting
- **107 tests** — comprehensive coverage across all features
- **Streaming TUI** — ratatui with markdown rendering, vim keybindings, live token display
- **AI workflows** — commit, review, explain, test analysis, watch mode

## Quick Start

```bash
# Clone and build
git clone https://github.com/dirmacs/pawan && cd pawan
cargo install --path crates/pawan-cli

# Set API key (NVIDIA NIM free tier)
export NVIDIA_API_KEY=nvapi-...

# Or use local Ollama (no API key needed)
export PAWAN_PROVIDER=ollama
export PAWAN_MODEL=llama3.2

# Interactive TUI
pawan

# AI-powered commit
pawan commit -a

# Self-heal: fix errors, warnings, test failures
pawan heal

# AI code review of current changes
pawan review

# Explain a file or concept
pawan explain src/lib.rs

# Run tests and AI-analyze failures
pawan test --fix

# Headless scripted execution
pawan run "add input validation to the config parser"

# Watch mode: auto-heal on file changes
pawan watch --interval 10

# Check setup
pawan doctor
```

## Command Reference

### 🎯 Core Commands

#### `pawan heal`
Auto-fix compilation errors, warnings, and test failures.

```bash
# Fix all issues
pawan heal

# Fix specific error types
pawan heal --errors    # Only compilation errors
pawan heal --warnings  # Only warnings
pawan heal --tests     # Only test failures
```

#### `pawan task "..."`
Execute a coding task with AI assistance.

```bash
# Simple task
pawan task "add user authentication to the API"

# With specific files
pawan task "refactor the database module" --files src/db.rs

# With context
pawan task "optimize query performance" --context "current implementation is slow"
```

#### `pawan commit [-a]`
AI-powered commit messages with optional auto-stage.

```bash
# Stage changes + generate message + commit
pawan commit -a

# Generate message only
pawan commit --message-only

# Custom message override
pawan commit -m "feat: add new feature"
```

#### `pawan improve <target>`
Improve code quality (documentation, refactoring, tests).

```bash
# Improve documentation
pawan improve docs

# Improve specific file
pawan improve src/parser.rs

# Full improvement
pawan improve --all
```

#### `pawan test [--fix]`
Run tests with AI analysis of failures.

```bash
# Run all tests
pawan test

# Run with auto-fix for failures
pawan test --fix

# Run specific test
pawan test src/lib.rs::test_function

# Show detailed output
pawan test --verbose
```

#### `pawan review [--staged]`
AI code review with severity-level feedback.

```bash
# Review staged changes
pawan review --staged

# Review all changes
pawan review

# Focus on specific issues
pawan review --security
pawan review --performance
pawan review --style
```

#### `pawan explain <query>`
AI explanation of files, functions, or concepts.

```bash
# Explain a file
pawan explain src/lib.rs

# Explain a function
pawan explain "how does the parser work?"

# Explain code block
pawan explain --code "snippet of code"
```

### ⚙️ Automation Commands

#### `pawan run "prompt"`
Headless single-prompt execution (for scripting).

```bash
# Simple prompt
pawan run "add input validation to the config parser"

# File-based prompt
pawan run -f prompt.md

# With timeout
pawan run "task" --timeout 300

# Save results
pawan run "task" --output result.md
```

#### `pawan watch [-i INTERVAL]`
Poll cargo check and auto-heal on file changes.

```bash
# Watch with 10s interval
pawan watch --interval 10

# Watch with verbose output
pawan watch --verbose

# Watch specific crate
pawan watch --crate pawan-core
```

### 📁 Project Commands

#### `pawan init`
Scaffold PAWAN.md + pawan.toml configuration.

```bash
# Initialize new project
pawan init

# With custom settings
pawan init --provider ollama --model llama3.2
```

#### `pawan doctor`
Diagnose setup (API keys, connectivity, tools).

```bash
# Full diagnosis
pawan doctor

# Check specific items
pawan doctor --keys
pawan doctor --connectivity
pawan doctor --tools
```

#### `pawan status`
Show project health summary.

```bash
# Full status
pawan status

# Quick status
pawan status --short
```

#### `pawan sessions`
List and manage saved sessions.

```bash
# List sessions
pawan sessions

# Resume session
pawan sessions --resume SESSION_ID

# Delete session
pawan sessions --delete SESSION_ID
```

### ⚙️ Configuration Commands

#### `pawan config show`
Display resolved configuration.

```bash
# Show full config
pawan config show

# Show specific section
pawan config show provider
pawan config show model
```

#### `pawan mcp list`
Show configured MCP servers and available tools.

```bash
# List all MCP servers
pawan mcp list

# List tools for specific server
pawan mcp list --server daedra
```

#### `pawan completions <shell>`
Generate shell completions.

```bash
# Bash
pawan completions bash

# Zsh
pawan completions zsh

# Fish
pawan completions fish
```

### 🎨 Interactive Mode

#### `pawan`
Launch interactive TUI with streaming markdown.

```bash
# Start TUI
pawan

# Resume saved session
pawan chat --resume SESSION_ID

# With specific model
pawan --model qwen/qwen3.5-397b-a17b
```

**TUI Features:**
- Markdown rendering with dark code blocks
- Streaming tokens as they arrive
- Tool execution progress notifications
- Vim keybindings: `/` search, `n`/`N` navigation, `g`/`G` scroll
- Token usage tracking in status bar

## Configuration

Pawan loads config in priority order: **CLI flags > env vars > pawan.toml > defaults**

### Environment Variables

```bash
# Provider (nvidia | ollama | openai)
export PAWAN_PROVIDER=nvidia

# Model (required)
export PAWAN_MODEL=mistralai/devstral-2-123b-instruct-2512

# Temperature (0.0-2.0, default: 0.8)
export PAWAN_TEMPERATURE=1.0

# Max tokens (default: 8192)
export PAWAN_MAX_TOKENS=8192

# API key (auto-loaded from .env if PAWAN_API_KEY not set)
export NVIDIA_API_KEY=nvapi-...
```

### pawan.toml Configuration

```toml
# Main settings
provider = "nvidia"
model = "mistralai/devstral-2-123b-instruct-2512"
temperature = 1.0
max_tokens = 8192

# MCP servers configuration
[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]

[mcp.wikipedia]
command = "wikipedia-mcp"
args = ["serve"]
```

### Working with Local Models (llama.cpp)

Use the OpenAI provider pointing at localhost:

```toml
# pawan.toml
provider = "openai"
model = "localhost:11434/llama3.2"
temperature = 0.8
```

```bash
# Run llama.cpp server in background
ollama serve llama3.2

# Or with llama.cpp directly
./build/bin/llama-server -m ./models/llama3.2.gguf -c 4096 -np 1 -sp 0.8 --host 0.0.0.0 --port 11434
```

Then use:

```bash
pawan --provider openai --model localhost:11434/llama3.2
```

## Architecture

```
pawan/
  crates/
    pawan-core/    # Library: agent engine, 17 tools, config, healing
    pawan-cli/     # Binary: CLI + ratatui TUI + AI workflows
    pawan-mcp/     # MCP client (rmcp 0.12, stdio transport)
    pawan-aegis/   # Aegis config resolution
```

`pawan-core` has zero internal dependencies — it works standalone with any OpenAI-compatible API.

## Ecosystem

| Tool | What |
|------|------|
| [aegis](https://github.com/dirmacs/aegis) | Declarative config management |
| [ares](https://github.com/dirmacs/ares) | Agentic retrieval-enhanced server |
| [daedra](https://github.com/dirmacs/daedra) | Web search MCP server |
| [nimakai](https://github.com/dirmacs/nimakai) | NIM model latency benchmarker |

## License

MIT
