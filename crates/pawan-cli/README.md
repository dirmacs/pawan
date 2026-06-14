# pawan v0.5.16

Pawan (पवन) — CLI coding agent with pluggable LLM backends, 37 tools, and cross-session memory.

## Unreleased

## What's New in v0.5.16

- **RMUX session-list cards** — completed `rmux list_sessions` tool calls render active-session inventory instead of raw JSON.

## What's New in v0.5.15

- **RMUX pane-list cards** — completed `rmux list_panes` tool calls render active-pane inventory instead of raw JSON.

## What's New in v0.5.14

- **`/rmux panes [session]`** — routes active RMUX pane discovery through the agent's `list_panes` tool action.

## What's New in v0.5.13

- **`/rmux list`** — routes active RMUX session discovery through the agent's `list_sessions` tool action.

## What's New in v0.5.12

- **RMUX snapshot cards** — completed `rmux` snapshot tool calls render pane metadata and visible terminal text instead of raw JSON.
- **Lazy TUI statics** — fixed-initializer TUI statics now use `std::sync::LazyLock`.

## What's New in v0.5.11

- **`/rmux kill <session>`** — routes explicit RMUX session teardown through the agent's `kill_session` tool action.

## What's New in v0.5.10

- **`/rmux` slash command** — routes durable terminal-multiplexer tasks through the agent's RMUX tool, with typed `session`, `send`, `key`, `wait`, and `snapshot` forms plus snapshot evidence expected before reporting.
- **Headless TUI QA** — PTY-backed integration test drives the real TUI and snapshots the rendered model picker with `vt100` + `insta`.
- **Model picker Enter fix** — key-release events are ignored before modal routing so `/model` does not auto-select the first model.

## What's New in v0.5.9

- **TUI redesign** — single-letter slash commands removed; token/ctx widget fixed for non-OpenAI providers; auto-scroll pinned to bottom; redesigned permission popup; framed tool-call cards with collapse/expand; rounded borders, branded `◆ pawan` title, badge-pill role headers
- **`animate-core` value tweens** — rolling token counts, eased ctx% bar, accent-colour fade on `/theme` switches (replaces hand-rolled `ColorTransition`)
- **tachyonfx cell effects** — content reveal, popup sweep-in, status pulse (suppressed under test)
- **tui-scrollview / ratatui-cheese / Rect::centered()** — automatic scrollbar, animated spinner, centered modals

## What's New in v0.5.8

- **Slash command reliability** — `/theme <name>` and other argument-bearing slash commands submit correctly when pressing Enter
- **Readable input placeholder** — textarea placeholder and reset paths use active theme colors instead of low-contrast defaults
- **Status bar polish** — model, tokens, context percentage/bar, iteration, and clock are separated and spaced clearly
- **Regression coverage** — added key-event and Ratatui TestBackend tests for `/theme`, placeholder styling, and status formatting
- **Test suite expansion** — 53 new TUI types tests covering format parsing, strip_reasoning_tags, ContentBlock, ToolBlockState; 1779 total workspace tests across 18 suites

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
