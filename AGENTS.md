# Pawan — Agent Context

Pawan (पवन) is a self-healing CLI coding agent built in Rust. 2-crate workspace. Used as the primary dirmacs coding agent for autonomous task execution.

## Architecture

```
pawan-core/    — Library (zero dirmacs deps). Lib name: pawan
  agent/       — PawanAgent, tool-calling loop, LlmBackend trait
    backend/   — openai_compat (NIM/OpenAI), ollama (local fallback)
  tools/       — Tool trait + ToolRegistry (file ops, bash, git, search)
  healing/     — Auto-repair: compile errors, test failures, warnings
  config/      — PawanConfig from pawan.toml (TOML + ${ENV_VAR} subs)

pawan-cli/     — Binary with clap CLI + ratatui TUI
```

## Common Tasks

**Add a new tool:**
1. Implement `tools::Tool` trait in `pawan-core/src/tools/`
2. Register in `ToolRegistry` in `tools/mod.rs`
3. Add integration test in `tests/`

**Add a new LLM backend:**
1. Implement `agent::backend::LlmBackend` trait
2. Add variant to backend dispatch in `agent/mod.rs`
3. Add config section to `pawan.toml` schema in `config.rs`

**Change self-heal behavior:**
- Healing loop lives in `pawan-core/src/healing/`
- Triggers on: cargo build failure, test failure, clippy warnings
- Configurable max iterations via `pawan.toml [agent] max_iterations`

## Key Decisions

- **pawan-core has zero dirmacs deps** — keeps it publishable to crates.io independently
- **NVIDIA NIM default** — `integrate.api.nvidia.com/v1`, OpenAI-compatible protocol
- **Ollama fallback** — for local/offline use, no NIM key required
- **TOML over JSON** — `pawan.toml` config, not JSON like openclaw used to be
- **ratatui TUI** — rich terminal UI in pawan-cli, not just plain stdout

## NIM Model Compatibility

| Model | Tool Use | Notes |
|-------|----------|-------|
| `nvidia/devstral-2-123b` | ✅ | Recommended for coding tasks |
| `qwen/qwen3-coder-480b-a35b` | ✅ | Strong coder, high latency |
| `stepfun-ai/step-3.5-flash` | ✅ | Fast, reliable |
| `deepseek-ai/deepseek-v3.2` | ⚠️ | Context drift on long sessions |
| `deepseek-ai/deepseek-r1` | ⚠️ | Reasoning model, slow for coding |

## Configuration

```toml
# pawan.toml
[providers.nvidia-nim]
base_url = "https://integrate.api.nvidia.com/v1"
api_key = "${NVIDIA_API_KEY}"

[[providers.nvidia-nim.models]]
id = "nvidia/devstral-2-123b"
name = "Devstral 2 123B"

[agent]
primary = "nvidia-nim/nvidia/devstral-2-123b"
fallbacks = ["nvidia-nim/stepfun-ai/step-3.5-flash"]
max_iterations = 30

[tools]
allow_bash = true
allow_network = false
```

## Pawan as a Library (dogfooding)

```rust
use pawan::{PawanAgent, PawanConfig};

let config = PawanConfig::load("pawan.toml")?;
let mut agent = PawanAgent::from_config(config)?;
agent.execute("Fix the compile errors in src/lib.rs").await?;
```

## Environment

- `NVIDIA_API_KEY` — NIM inference (auto-loaded from `.env` via dotenvy)
- `OLLAMA_BASE_URL` — local Ollama fallback (optional, default: `http://localhost:11434`)
- `RUST_LOG` — tracing log filter (default: `info`)
