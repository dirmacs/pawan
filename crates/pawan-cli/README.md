# pawan v0.3.2

Pawan (पवन) — CLI coding agent with pluggable LLM backends, 34 tools, and cross-session memory.

## What's New in v0.3.2

- **Comprehensive TUI Testing** — 79 tests covering all TUI functionality
- **Enhanced Model Selector** — Interactive model selection with search and filtering
- **Session Browser** — Browse, load, and manage saved sessions with sorting modes
- **Auto-save** — Automatic session saving at configurable intervals
- **Extended Slash Commands** — `/sessions`, `/save`, `/load`, `/resume`, `/new`
- **Improved Modal Rendering** — Better modal components for model selector and session browser
- **Enhanced Keyboard Handling** — Improved navigation and modal state management

## Install

```bash
cargo install pawan
```

## Features

- **Multi-model** — NVIDIA NIM, Ollama, OpenAI, MLX backends
- **34 tools** in 3 tiers (Core: file ops, Standard: git/search, Extended: web/MCP)
- **Ratatui TUI** with interleaved content blocks (tool calls inline with text)
- **Model Selector** — Interactive model selection with search and filtering
- **Session Browser** — Browse, load, and manage saved sessions
- **Auto-save** — Automatic session saving at configurable intervals
- **MCP client** via rmcp — connect to any MCP server
- **Eruka memory** — cross-session context via Eruka integration
- **Thinking modes** — per-model dispatch (Qwen, Gemma, Mistral, DeepSeek)

## Usage

```bash
# Interactive mode
pawan

# With a specific model
pawan --model qwen/qwen3.5-122b-a10b

# Execute a task
pawan -e "fix the failing test in src/lib.rs"

# Use new slash commands
pawan  # then use /sessions, /save, /load, /resume, /new
```

## License

MIT