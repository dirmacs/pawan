# Pawan

Rust 5-crate workspace: `pawan-core` (library), `pawan-mcp`, `pawan-web`, `pawan-aegis`, `pawan-cli` (binary with TUI).

## Build & Test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Architecture

- `pawan-core`: Agent engine, tools, config, healing. Zero dirmacs deps. Lib name is `pawan`.
- `pawan-mcp`: MCP client integration (rmcp, stdio transport).
- `pawan-web`: HTTP API server (Axum + SSE, port 3300).
- `pawan-aegis`: Aegis config resolution.
- `pawan-cli`: CLI binary + ratatui TUI. Depends on pawan-core.

## Conventions

- NVIDIA NIM as default provider (`https://integrate.api.nvidia.com/v1`)
- OpenAI-compatible protocol for all providers
- Tool trait + ToolRegistry pattern for extensibility
- Config via `pawan.toml` (TOML)
- Git author: bkataru, not root
