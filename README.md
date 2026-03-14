<p align="center">
  <strong>पवन — Self-healing CLI coding agent</strong>
</p>

<p align="center">
  <a href="https://github.com/dirmacs/pawan/actions"><img src="https://github.com/dirmacs/pawan/workflows/CI/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/tools-17-green.svg" alt="17 tools">
  <img src="https://img.shields.io/badge/tests-107-brightgreen.svg" alt="107 tests">
</p>

---

**Pawan** (पवन, "wind") is a Rust-native CLI coding agent. It reads, writes, and heals code autonomously — no subscription, no vendor lock-in. Built for the [dirmacs](https://github.com/dirmacs) ecosystem, works with any OpenAI-compatible API.

## Why Pawan

- **Self-hosted** — runs on your own machine, uses your own API keys
- **17 tools** — file ops, search, bash, git (status/diff/add/commit/log/blame/branch/checkout/stash), sub-agents
- **18 subcommands** — from interactive TUI to headless scripting
- **107 tests** — markdown rendering, API error handling, CLI parsing, git tools, integration
- **Streaming TUI** — ratatui with markdown rendering, vim keybindings, live token display
- **AI workflows** — commit, review, explain, test analysis, watch mode

## Quick Start

```bash
# Clone and build
git clone https://github.com/dirmacs/pawan && cd pawan
cargo install --path crates/pawan-cli

# Set API key (NVIDIA NIM free tier)
export NVIDIA_API_KEY=nvapi-...

# Interactive TUI
pawan

# AI-powered commit (stage + generate message + commit)
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

## Subcommands

| Category | Command | Description |
|----------|---------|-------------|
| **Interactive** | `pawan` | TUI chat with streaming + markdown |
| | `pawan chat --resume ID` | Resume a saved session |
| **Code** | `pawan heal` | Auto-fix compilation errors, warnings, tests |
| | `pawan task "..."` | Execute a coding task |
| | `pawan commit -a` | AI commit: stage, generate message, commit |
| | `pawan improve docs` | Improve code (docs, refactor, tests) |
| | `pawan test --fix` | Run tests, AI-analyze + fix failures |
| | `pawan review --staged` | AI code review with severity levels |
| | `pawan explain <query>` | AI explanation of files/functions |
| **Automation** | `pawan run "prompt"` | Headless single-prompt execution |
| | `pawan run -f prompt.md` | File-based prompt |
| | `pawan watch -i 10` | Poll cargo check, auto-heal loop |
| **Project** | `pawan init` | Scaffold PAWAN.md + pawan.toml |
| | `pawan doctor` | Diagnose setup (keys, connectivity, tools) |
| | `pawan status` | Project health summary |
| | `pawan sessions` | List saved sessions |
| **Config** | `pawan config show` | Display resolved config |
| | `pawan mcp list` | Show MCP servers and tools |
| | `pawan completions bash` | Generate shell completions |

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

## Configuration

Pawan loads config in priority order: **CLI flags > env vars > pawan.toml > defaults**

```bash
# Environment variables
export PAWAN_MODEL=qwen/qwen3.5-397b-a17b
export PAWAN_PROVIDER=nvidia    # nvidia | ollama | openai
export PAWAN_TEMPERATURE=0.8
export PAWAN_MAX_TOKENS=8192
```

```toml
# pawan.toml
provider = "nvidia"
model = "mistralai/devstral-2-123b-instruct-2512"
temperature = 1.0
max_tokens = 8192

[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

Add `PAWAN.md` to your project root for per-project context (like CLAUDE.md).

## TUI Features

- Markdown rendering: headers, **bold**, *italic*, `code`, code blocks with dark bg, bullets, numbered lists, blockquotes
- Streaming tokens (appear as they arrive)
- Tool execution progress (start/complete notifications)
- Vim keybindings: `/` search, `n`/`N` next/prev, `g`/`G` top/bottom, `Ctrl+u`/`d` half-page
- Token usage tracking in status bar
- Mouse scroll support

## Ecosystem

| Tool | What |
|------|------|
| [aegis](https://github.com/dirmacs/aegis) | Declarative config management |
| [ares](https://github.com/dirmacs/ares) | Agentic retrieval-enhanced server |
| [daedra](https://github.com/dirmacs/daedra) | Web search MCP server |
| [nimakai](https://github.com/dirmacs/nimakai) | NIM model latency benchmarker |

## License

MIT
