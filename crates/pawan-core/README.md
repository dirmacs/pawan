# pawan-core

Core library for the Pawan CLI coding agent. Contains the agent engine, tool system, configuration, and healing/recovery logic.

## Features

- **Agent Engine** — Multi-turn conversation with LLM backends (NVIDIA NIM, Ollama, OpenAI, MLX)
- **Tool System** — 29 tools in 3 tiers (Core/Standard/Extended) with tiered visibility
- **Configuration** — Pluggable config resolution via aegis or local files
- **Healing** — Auto-recovery from tool failures, context overflow, model errors
- **Thinking Budget** — Per-model thinking mode dispatch (Qwen, Gemma, Mistral, DeepSeek)

## Usage

```rust
use pawan_core::{Agent, Config};

let config = Config::load()?;
let agent = Agent::new(config).await?;
```

This crate is the foundation — use `pawan` (the CLI binary) for the full experience.

## License

MIT
