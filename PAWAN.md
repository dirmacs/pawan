# Pawan Project Context

Rust 5-crate workspace:
- `pawan-core`: Library (agent engine, tools, config, healing). Lib name is `pawan`.
- `pawan-mcp`: MCP client integration (thulp-mcp, stdio transport).
- `pawan-web`: HTTP API server (Axum + SSE streaming, port 3300).
- `pawan-aegis`: Aegis config resolution — generates `pawan.toml` from Aegis manifests.
- `pawan-cli`: Binary with clap CLI and ratatui TUI.

## Architecture

- `agent::backend::LlmBackend` trait abstracts LLM providers
- `agent::backend::openai_compat::OpenAiCompatBackend` for NVIDIA NIM / OpenAI
- `agent::backend::ollama::OllamaBackend` for local Ollama
- `tools::Tool` trait + `ToolRegistry` for tool registration
- Config loads from `pawan.toml` in cwd

## Build & Test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Conventions

- Default provider: NVIDIA NIM (`https://integrate.api.nvidia.com/v1`)
- API key from env: `NVIDIA_API_KEY` (auto-loaded from `.env` via dotenvy)
- Git author: bkataru, not root
- Keep pawan-core free of dirmacs-internal dependencies
