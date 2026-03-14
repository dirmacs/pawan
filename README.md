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
    pawan-cli/     # Binary: CLI + ratatui TUI
    pawan-mcp/     # MCP client integration (rmcp 0.12)
    pawan-aegis/   # Aegis config generation
```

**pawan-core** has zero dirmacs-internal dependencies — it works standalone with any OpenAI-compatible API.

## Configuration

Create `pawan.toml` in your project root (or copy `pawan.example.toml`):

```toml
model = "mistralai/devstral-2-123b-instruct-2512"
temperature = 0.6

[healing]
fix_errors = true
fix_warnings = true
fix_tests = true

[permissions]
# write_file = "deny"   # Disable file writing
# bash = "deny"          # Disable shell execution

[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

Add a `PAWAN.md` file to your project root for per-project context (like CLAUDE.md).

## Subcommands

| Command | Description |
|---------|-------------|
| `pawan` | Interactive TUI chat (default) |
| `pawan chat [--resume ID]` | Chat mode, optionally resume a session |
| `pawan run "prompt"` | Headless single-prompt execution |
| `pawan run -f prompt.md -o json` | File-based prompt, JSON output |
| `pawan heal` | Auto-fix errors, warnings, and failing tests |
| `pawan task "..."` | Execute a specific coding task |
| `pawan commit` | Generate and apply a commit message |
| `pawan improve docs\|refactor\|tests\|all` | Improve code quality |
| `pawan status` | Show project health summary |
| `pawan sessions` | List saved sessions |
| `pawan mcp list` | Show connected MCP servers and tools |
| `pawan config show` | Display resolved configuration |
| `pawan config init` | Generate pawan.toml template |

## Tools

15 built-in tools + dynamic MCP tools:

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
| `git_log` | View commit history |
| `git_blame` | Line-by-line authorship |
| `git_branch` | List and show branches |
| `spawn_agent` | Spawn a sub-agent for delegated tasks |

## Dirmacs Ecosystem

Pawan is the foundational coding agent in the dirmacs toolchain:

- **[aegis](https://github.com/dirmacs/aegis)** — Declarative config management
- **[ares](https://github.com/dirmacs/ares)** — Agentic retrieval-enhanced server
- **[daedra](https://github.com/dirmacs/daedra)** — Web search MCP server
- **[nimakai](https://github.com/dirmacs/nimakai)** — NIM model latency benchmarker

## License

MIT
