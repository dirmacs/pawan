# pawan v0.5.0

Pawan (पवन) — CLI coding agent with pluggable LLM backends, 34 tools, and cross-session memory.

## What's New in v0.5.0

- **TUI overhaul** — 12 ratatui components: theme, splash, highlight, layout, status_bar, scrollbar, activity_panel, queue_panel, tool_display, render, app, slash_commands
- **Animated theme transitions** — `ColorTransition::set()` animates accent color on `/theme` switch; `⚡` indicator during transition
- **StatusBar component** — mode badge (INPUT/NORMAL/CMD/HELP/MODEL), context bar, flash-on-event for UI events, iteration counter, timestamp
- **Session store** — SQLite in WAL mode with FTS5 and JSON migration; JSONL session branching (parent_id, depth cap 5)
- **Agent pool** — concurrent agents with semaphore bounding; 6 agent types, 300s timeout
- **Parallel tool execution** — bounded concurrency; batch tool (25 concurrent calls)
- **Bash permission tiers** — tree-sitter based, feature-gated
- **Doom-loop detection** — configurable backoff; retry policy with exponential backoff + jitter
- **CLI flags** — `--print` headless, `--output-format` (text/json/stream-json), `--continue`, `--session`, `--list-sessions`
- **Keybind contexts + model picker modal** (`Ctrl+M`); fuzzy search modal (`Ctrl+P`)
- **208 tests** — 173 TUI + 35 CLI passing

## What's New in v0.4.8

- **Shell-like command history navigation** — Up/down arrow keys navigate through command history, matching bash/zsh/Claude Code/OMP behavior
- **History persistence** — Commands saved to history (excluding slash commands), accessible across sessions

## What's New in v0.4.7

- **Redesigned keyboard shortcuts** — Ctrl+C always clears input, Ctrl+Q quits for cleaner separation of concerns

## What's New in v0.4.6

- **Redesigned Ctrl+C behavior** — Clears input when non-empty, quits when empty (matching bash/zsh pattern)

## What's New in v0.4.1

- **Session tags UI** — Visual green tags in status bar, manage via `/tag` command
- **Fuzzy session search** — Fuzzy matching with `[FUZZY]` indicator in session browser
- **NVIDIA NIM catalog** — `/models` command to browse available NIM models
- **Enhanced `/diff`** — `--cached` flag support and colorized diff output
- **Improved `/load` and `/resume`** — Opens session browser when called without arguments
- **Enhanced scrolling** — PageUp/PageDown/Home/End keys and mouse wheel support in all popups (command palette, slash menu, session browser)
- **Secure credential storage** — API keys stored in OS-native credential store (Keychain/Credential Manager/libsecret)
- **Test stability** — All 722 library tests and 59 integration tests now pass consistently

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
