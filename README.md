<p align="center">
  <img src="docs/img/pawan-logo.svg" width="128" alt="pawan">
</p>

<h1 align="center">पवन — pawan</h1>

<p align="center">
  Self-healing CLI coding agent. Rust. 22+ tools. Runs on your hardware.<br>
  No subscription. No telemetry. No vendor lock-in.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/tools-22+-green.svg" alt="22+ tools">
  <img src="https://img.shields.io/badge/tests-147-brightgreen.svg" alt="147 tests">
</p>

---

Pawan reads, writes, and heals code. It has a tool-calling loop, streaming TUI, git integration, and works with any OpenAI-compatible API — NVIDIA NIM, Ollama, llama.cpp, or your own endpoint.

Built by [DIRMACS](https://dirmacs.com). Named after the Hindi word for wind (पवन).

## Install

```bash
cargo install pawan

# Or from source
git clone https://github.com/dirmacs/pawan && cd pawan
cargo install --path crates/pawan-cli
```

```bash
# NVIDIA NIM (free tier)
export NVIDIA_API_KEY=nvapi-...
pawan

# Local Ollama (no key needed)
PAWAN_PROVIDER=ollama PAWAN_MODEL=llama3.2 pawan
```

## What it does

```bash
pawan                  # interactive TUI with streaming markdown
pawan heal             # auto-fix compilation errors, warnings, test failures
pawan task "..."       # execute a coding task
pawan commit -a        # AI-generated commit messages
pawan review           # AI code review of current changes
pawan test --fix       # run tests, AI-analyze and fix failures
pawan explain src/x.rs # explain code
pawan run "prompt"     # headless single-prompt (for scripting)
pawan watch -i 10      # poll cargo check, auto-heal on errors
pawan tasks ready      # show actionable unblocked beads
pawan doctor           # diagnose setup issues
```

## Tools (22+)

File ops (read, write, edit, glob, grep), bash execution, git (status, diff, add, commit, log, blame, branch, checkout, stash), sub-agent spawning, search (ripgrep with regex). All tool calls are streamed to the TUI in real-time.

## Architecture

```
pawan/
  crates/
    pawan-core/    # library — agent engine, 22+ tools, config, healing
    pawan-cli/     # binary — CLI + ratatui TUI + AI workflows
    pawan-web/     # HTTP API — Axum SSE server (port 3300)
    pawan-mcp/     # MCP client (rmcp 0.12, stdio transport)
    pawan-aegis/   # aegis config resolution
```

### Recent additions (2026-03-19)

**Git-backed sessions** — conversations stored as git commits in a bare repo. Fork from any point, list leaves (conversation tips), walk lineage. Inspired by [Karpathy's AgentHub](https://github.com/karpathy/agenthub).

**Beads task tracking** — hash-based IDs (`bd-a1b2c3d4`), dependency graphs, `ready()` detection, memory decay. Inspired by [Yegge's Beads](https://github.com/steveyegge/beads). CLI: `pawan tasks list/ready/create/close/dep/decay`.

**Eruka bridge** — injects [Eruka](https://eruka.dirmacs.com) 3-tier memory (Core/Working/Archival) into agent context before every LLM call. Archives completed sessions back to Eruka.

**Multi-agent identity** — each pawan instance gets an identity derived from its [aegis-net](https://github.com/dirmacs/aegis) peer name (e.g. `pawan@vps`). Foundation for swarm coordination.

**pawan-web** — Axum HTTP server with SSE streaming. `POST /api/chat/stream` maps the TUI's `AgentEvent` pattern to server-sent events. Sessions CRUD, model listing, agent identity. Deployed at `pawan.dirmacs.com`.

## Configuration

Priority: CLI flags > env vars > `pawan.toml` > defaults

```bash
PAWAN_PROVIDER=nvidia          # nvidia | ollama | openai
PAWAN_MODEL=stepfun-ai/step-3.5-flash
PAWAN_TEMPERATURE=1.0
PAWAN_MAX_TOKENS=8192
NVIDIA_API_KEY=nvapi-...
```

```toml
# pawan.toml
provider = "nvidia"
model = "stepfun-ai/step-3.5-flash"
temperature = 1.0
fallback_models = ["mistralai/devstral-2-123b-instruct-2512"]

[eruka]
enabled = true
url = "http://localhost:8081"

[cloud]
provider = "nvidia"
model = "mistralai/devstral-2-123b-instruct-2512"

[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

## Hybrid routing

Pawan supports local-first inference with cloud fallback:

1. **Local** (primary) — Ollama / llama.cpp / llama-server on your machine or via aegis-net tunnel to a Mac with Apple Silicon
2. **Cloud** (fallback) — NVIDIA NIM when local is unavailable

Zero-cost local inference with cloud reliability as a safety net.

## Ecosystem

| Project | What |
|---------|------|
| [ares](https://github.com/dirmacs/ares) | Agentic retrieval-enhanced server (RAG, embeddings, multi-provider LLM) |
| [eruka](https://github.com/dirmacs/eruka) | Context intelligence engine (knowledge graph, memory tiers, decay) |
| [aegis](https://github.com/dirmacs/aegis) | Config management + WireGuard mesh overlay (aegis-net) |
| [doltares](https://github.com/dirmacs/doltares) | Orchestration daemon (DAG workflows, council/consultant nodes) |
| [doltclaw](https://github.com/dirmacs/doltclaw) | Minimal Rust agent runtime |
| [nimakai](https://github.com/dirmacs/nimakai) | NIM model latency benchmarker (Nim) |
| [daedra](https://github.com/dirmacs/daedra) | Web search MCP server |

## License

MIT
