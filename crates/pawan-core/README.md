# pawan-core v0.3.2

Core library for the Pawan CLI coding agent. Contains the agent engine, tool system, configuration, and healing/recovery logic.

## What's New in v0.3.2

- **Enhanced TUI Testing** — Comprehensive test suite with 79 tests covering all TUI functionality
- **Model Selector** — Interactive model selection with search and filtering capabilities
- **Session Browser** — Browse, load, and manage saved sessions with sorting modes
- **Auto-save** — Automatic session saving at configurable intervals
- **Slash Commands** — Extended command set: `/sessions`, `/save`, `/load`, `/resume`, `/new`
- **Modal Rendering** — Improved modal components for model selector and session browser
- **Keyboard Handling** — Enhanced keyboard navigation and modal state management

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
use pawan_core::{Agent, Config};

let config = Config::load()?;
let agent = Agent::new(config).await?;
```

This crate is the foundation — use `pawan` (the CLI binary) for the full experience.

## License

MIT