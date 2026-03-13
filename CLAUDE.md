# Pawan

Rust workspace: `pawan-core` (library) + `pawan-cli` (binary with TUI).

## Build & Test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Architecture

- `pawan-core`: Agent engine, tools, config, healing. Zero dirmacs deps. Lib name is `pawan`.
- `pawan-cli`: CLI binary + ratatui TUI. Depends on pawan-core.

## Conventions

- NVIDIA NIM as default provider (`https://integrate.api.nvidia.com/v1`)
- OpenAI-compatible protocol for all providers
- Tool trait + ToolRegistry pattern for extensibility
- Config via `pawan.toml` (TOML)
- Git author: bkataru, not root
