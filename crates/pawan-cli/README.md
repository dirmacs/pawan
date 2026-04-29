# pawan v0.5.6

Pawan (पवन) — CLI coding agent with pluggable LLM backends, 34 tools, and cross-session memory.

## What's New in v0.5.6

- **Slash command reliability** — `/theme <name>` and other argument-bearing slash commands submit correctly when pressing Enter
- **Readable input placeholder** — textarea placeholder and reset paths use active theme colors instead of low-contrast defaults
- **Status bar polish** — model, tokens, context percentage/bar, iteration, and clock are separated and spaced clearly
- **Regression coverage** — added key-event and Ratatui TestBackend tests for `/theme`, placeholder styling, and status formatting
- **Test suite expansion** — 53 new TUI types tests covering format parsing, strip_reasoning_tags, ContentBlock, ToolBlockState; 988+ total workspace tests with 74.58% line coverage

## What's New in v0.5.0

- **TUI overhaul** — 11 ratatui components: theme, splash, highlight, layout, status_bar, scrollbar, queue_panel, tool_display, render, app, slash_commands
- **Animated theme transitions** — `ColorTransition::set()` animates accent color on `/theme` switch; `⚡` indicator during transition
- **StatusBar component** — mode badge (INPUT/NORMAL/CMD/HELP/MODEL), context bar, flash-on-event for UI events, iteration counter, timestamp
- **Session store** — SQLite in WAL mode with FTS5 and JSON migration; JSONL session branching (parent_id, depth cap 5)
- **Agent pool** — concurrent agents with semaphore bounding; 6 agent types, 300s timeout
- **Parallel tool execution** — bounded concurrency; batch tool (25 concurrent calls)
- **Bash permission tiers** — tree-sitter based, feature-gated
- **Doom-loop detection** — configurable backoff; retry policy with exponential backoff + jitter
- **CLI flags** — `--print` headless, `--output-format` (text/json/stream-json), `--continue`, `--session`, `--list-sessions`
- **Keybind contexts + model picker modal** (`Ctrl+M`); fuzzy search modal (`Ctrl+P`)
- **Full CLI/TUI test suite** passing before release

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
- **MCP client** via thulp-mcp — connect to any MCP server
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
