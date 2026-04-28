# pawan-core v0.5.4

Core library for the Pawan CLI coding agent. Contains the agent engine, tool system, configuration, and healing/recovery logic.

## What's New in v0.5.4

- **Release alignment** — workspace crates now publish together at `0.5.4` with the TUI contrast and slash-picker fixes in the CLI

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
- **Full core test suite** passing before release

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
