<p align="center">
  <img src="docs/img/pawan-logo.svg" width="128" alt="pawan">
</p>

<h1 align="center">पवन — pawan</h1>

<p align="center">
  Self-healing CLI coding agent. Rust. 29 tools. AST + LSP powers. Runs on your hardware.<br>
  No subscription. No telemetry. No vendor lock-in.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/tools-29-green.svg" alt="29 tools">
  <img src="https://img.shields.io/badge/tests-372-brightgreen.svg" alt="372 tests">
</p>

---

Pawan reads, writes, and heals code. It has a tool-calling loop, streaming TUI, git integration, AST-level code rewriting, and works with any OpenAI-compatible API — NVIDIA NIM, MLX, Ollama, or your own endpoint.

Built by [DIRMACS](https://dirmacs.com). Named after [Power Star Pawan Kalyan](https://en.wikipedia.org/wiki/Pawan_Kalyan) — martial artist, Telugu cinema icon, Deputy CM of Andhra Pradesh. That energy: raw power, cult following, fearless execution.

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

# Local MLX on Mac (no key needed, $0 inference)
# Start mlx_lm.server, then:
PAWAN_PROVIDER=mlx pawan

# Local Ollama
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

## Tools (28)

| Category | Tools |
|----------|-------|
| **File** | read, write, edit (anchor-mode + string-replace), insert_after, append, list_directory |
| **Search** | glob, grep, ripgrep (native rg), fd (native) |
| **Code Intelligence** | **ast_grep** — AST-level structural search and rewrite via tree-sitter |
| **Shell** | bash, sd (find-replace), tree (erdtree), mise (runtime manager), zoxide |
| **Git** | status, diff, add, commit, log, blame, branch, checkout, stash |
| **Agent** | spawn_agent, spawn_agents (parallel sub-agents) |
| **MCP** | Dynamic tool discovery from any MCP server |

### ast-grep — structural code manipulation

```bash
# Find all unwrap() calls across the codebase
ast_grep(action="search", pattern="$EXPR.unwrap()", lang="rust", path="src/")

# Replace them with ? operator in one shot
ast_grep(action="rewrite", pattern="$EXPR.unwrap()", rewrite="$EXPR?", lang="rust", path="src/")
```

Matches by syntax tree structure, not text. `$VAR` for single-node wildcards, `$$$VAR` for variadic.

## Architecture

```
pawan/
  crates/
    pawan-core/    # library — agent engine, 28 tools, config, healing
    pawan-cli/     # binary — CLI + ratatui TUI + AI workflows
    pawan-web/     # HTTP API — Axum SSE server (port 3300)
    pawan-mcp/     # MCP client (rmcp 0.12, stdio transport)
    pawan-aegis/   # aegis config resolution
  grind/           # autonomous data structure workspace (16 structures, 119 tests)
```

### Safety & intelligence features

- **Compile-gated confidence** — auto-runs `cargo check` after writing `.rs` files, injects errors back for self-correction
- **Path normalization** — detects and corrects double workspace prefix bug in all file tools
- **Token budget tracking** — separates thinking tokens from action tokens per call, visible in TUI (`think:130 act:270`)
- **Iteration budget awareness** — warns model when 3 tool iterations remain
- **Think-token stripping** — strips `<think>...</think>` from content and tool arguments

### Recent additions (2026-03-20)

**Mistral Small 4 119B** — new primary model. First to achieve 100% autonomous data structure score (interval tree 6/6). Self-refactored 9 callsites in its own native.rs. 11 NIM models benchmarked.

**ast-grep + LSP tools** — AST-level code search/rewrite (multi-language) and rust-analyzer powered type-aware intelligence. Structural refactors in one tool call.

**Token budget system** — `reasoning_tokens` and `action_tokens` tracked per LLM call. `thinking_budget` config caps thinking overhead. TUI shows `think:N act:N` split.

**Auto-install + tiered registry** — missing CLI tools auto-install via mise on first use. 29 tools in 3 tiers (Core/Standard/Extended) — ~40% prompt token savings.

**16 grind structures** — bloom filter, fenwick, skip list, trie, segment tree, DSU, treap, suffix array, leftist heap, radix tree, pairing heap, splay tree, rope, AVL tree, LRU cache, interval tree.

## Configuration

Priority: CLI flags > env vars > `pawan.toml` > `~/.config/pawan/pawan.toml` > defaults

```bash
PAWAN_PROVIDER=nvidia           # nvidia | ollama | openai | mlx
PAWAN_MODEL=mistralai/mistral-small-4-119b-2603
PAWAN_TEMPERATURE=0.6
PAWAN_MAX_TOKENS=4096
NVIDIA_API_KEY=nvapi-...
```

```toml
# pawan.toml
provider = "nvidia"
model = "mistralai/mistral-small-4-119b-2603"
temperature = 0.6
max_tokens = 4096
max_tool_iterations = 20
thinking_budget = 0

[cloud]
provider = "nvidia"
model = "stepfun-ai/step-3.5-flash"

[eruka]
enabled = true
url = "http://localhost:8081"

[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

## Hybrid routing

Pawan supports local-first inference with cloud fallback:

1. **Local** (primary) — MLX on Mac M4 / Ollama / llama.cpp — $0/token
2. **Cloud** (fallback) — NVIDIA NIM StepFun Flash — automatic failover when local is down

Zero-cost local inference with cloud reliability as a safety net.

## Model triage (11 models tested)

| Model | Provider | Coding | Tool Calling | Notes |
|-------|----------|--------|-------------|-------|
| **Mistral Small 4 119B** | NIM | **Best** (6/6 interval tree) | Good | First 100% autonomous score. Self-corrects via semantic reasoning. |
| **StepFun Flash** | NIM | Good | **Best** (98.9%) | Best for orchestration and multi-step tool chains. |
| MiniMax M2.5 | NIM | Good (4/5 B-Tree) | Good | Tied with Mistral on B-Tree, fewer tool calls. |
| Qwen3.5-9B-OptiQ | MLX | Execution only | 85% | Best local. 17-18 tok/s, $0. Can't generate complex algos. |

Full triage: [dirmacs.github.io/pawan/triage/](https://dirmacs.github.io/pawan/triage/)

## Ecosystem

| Project | What |
|---------|------|
| [ares](https://github.com/dirmacs/ares) | Agentic retrieval-enhanced server (RAG, embeddings, multi-provider LLM) |
| [eruka](https://eruka.dirmacs.com) | Context intelligence engine (knowledge graph, memory tiers, decay) |
| [aegis](https://github.com/dirmacs/aegis) | Config management + WireGuard mesh overlay (aegis-net) |
| [doltares](https://github.com/dirmacs/doltares) | Orchestration daemon (DAG workflows, council/consultant nodes) |
| [doltclaw](https://github.com/dirmacs/doltclaw) | Minimal Rust agent runtime |
| [nimakai](https://github.com/dirmacs/nimakai) | NIM model latency benchmarker (Nim) |
| [daedra](https://dirmacs.github.io/daedra) | Self-contained web search MCP server (7 backends, automatic fallback) |

## License

MIT
