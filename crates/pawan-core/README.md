# pawan-core v0.5.0

Core library for the Pawan CLI coding agent. Contains the agent engine, tool system, configuration, and healing/recovery logic.

## What's New in v0.5.0

- **Session store** — SQLite in WAL mode with FTS5 and JSON migration; JSONL branching with `parent_id` (depth capped at 5); session labels and bookmarks
- **Agent pool** — concurrent agents with semaphore bounding; agent definitions with YAML frontmatter; 6 agent types, 300s timeout
- **Parallel tool execution** — bounded concurrency (`max_parallel_tools`); batch tool (25 concurrent calls)
- **Bash permission tiers** — tree-sitter based, feature-gated with main/sub/lua audience bitflags
- **Doom-loop detection** — configurable backoff multiplier; retry policy with exponential backoff + jitter
- **Auto-compaction** — LLM summarization via `/compact`; strategies: default (10 msgs), aggressive (5 msgs), conservative (20 msgs)
- **Memory system** — consolidation (Jaccard similarity), retrieval, prompt injection scanner (6 patterns), `SessionScopedMemory` fencing
- **Tool registry overhaul** — `Tool::execute` now async; `on_pre_compress` hook for context pre-processing; `sync_turn` on return
- **Module splits** — `coordinator/types.rs` extracted; `tools/git.rs` → `tools/git/` (5 files); `tools/native.rs` split into `native_search`, `mise`, `lsp_tool`
- **208 tests** passing (173 library + 35 CLI integration)

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
- **Test stability** — All 789 library tests and 59 integration tests now pass consistently

## Features

- **Agent Engine** — Multi-turn conversation with LLM backends (NVIDIA NIM, Ollama, OpenAI, MLX)
- **Tool System** — 34 tools in 3 tiers (Core/Standard/Extended) with tiered visibility
- **Configuration** — Pluggable config resolution via aegis or local files
- **Healing** — Auto-recovery from tool failures, context overflow, model errors
- **Thinking Budget** — Per-model thinking mode dispatch (Qwen, Gemma, Mistral, DeepSeek)
- **Session Management** — Save, load, and resume conversation sessions
- **Model Selection** — Dynamic model switching with search and filtering

## Usage

```rust
use pawan::{PawanAgent, PawanConfig};
use std::path::PathBuf;

let config = PawanConfig::load(None)?;
let _agent = PawanAgent::new(config, PathBuf::from("."));
```

This crate is the foundation — use `pawan` (the CLI binary) for the full experience.

## License

MIT
