# pawan-core v0.4.8

Core library for the Pawan CLI coding agent. Contains the agent engine, tool system, configuration, and healing/recovery logic.

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
