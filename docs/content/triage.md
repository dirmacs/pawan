+++
title = "Model Triage"
+++

Pawan is model-agnostic — it works with any OpenAI-compatible API. We've triaged models on NVIDIA NIM and local inference for real-world coding agent tasks: tool calling, multi-step reasoning, code generation, and self-healing.

## Triage Results (updated 2026-04-12)

Tested across 1000+ cumulative tool calls, 16 data structure builds, and 372 tests across the workspace. 12 NIM models benchmarked via latency (nimakai) and real-world dogfooding (pawan task).

### Tier 1 — Production Ready

| Model | Provider | Latency | Task Time | Notes |
|-------|----------|---------|-----------|-------|
| **Qwen3.5 122B A10B** | NIM | 383ms | **13.6s** | Primary model. Fastest task completion, solid tool calling, thinking mode support. S tier (66% SWE). |
| **MiniMax M2.5** | NIM | 374ms | 73.8s | Cloud fallback. Highest SWE-bench score (80.2%). Best analysis quality but slower. |
| **Step 3.5 Flash** | NIM | 379ms | — | S+ tier (74.4% SWE). Fast latency but produced empty responses in dogfooding — needs investigation. |

### Tier 2 — Usable with Guardrails

| Model | Provider | Notes |
|-------|----------|-------|
| **Kimi K2 Thinking** | NIM | 470ms. Strong reasoning model but thinking mode overhead slows agentic tasks. |
| **Kimi K2 Instruct 0905** | NIM | 458ms. No thinking overhead, decent tool calling. |
| **Mistral Large 3 675B** | NIM | 685ms. Capable but slow for agent loops. |
| **GLM-4.7** | NIM | 1614ms. Strong benchmarks but too slow for real-time agent use. |

### Tier 3 — Avoid for Agents

| Model | Provider | Issue |
|-------|----------|-------|
| **Mistral Small 4 119B** | NIM | 400 error: "Unexpected role 'user' after role 'tool'" — Eruka context injection breaks Mistral's strict message ordering. |
| **Gemma 4 31B IT** | NIM | Thinking mode stalls pawan (15+ min with no tool calls). 9 TPS too slow for agentic tasks. |
| **GLM-5** | NIM | 8313ms latency. Unstable. |
| **DeepSeek V3.2** | NIM | Timeout in latency benchmark. |
| **Kimi K2.5** | NIM | Timeout in latency benchmark. |
| **Qwen3.5 397B A17B** | NIM | 404 / timeout — not reliably available. |

## Guardrails That Make It Work

Pawan's state machine includes guardrails that boost success rates across all models:

| Guardrail | What It Does |
|-----------|-------------|
| **Empty response nudge** | If model returns empty content + no tool calls, sends a nudge prompt to retry |
| **Repeat detection** | Detects when model repeats the same response 3x and forces a different approach |
| **Chatty no-op detection** | If model returns verbose planning text but no tool calls, nudges it to use tools |
| **Think-token stripping** | Strips `<think>...</think>` from content AND tool call arguments (StepFun/Qwen compat) |
| **UTF-8 safe truncation** | Truncates at char boundaries, not byte boundaries — no panics on multi-byte chars |
| **Resilient LLM retry** | Exponential backoff (2s, 4s, 8s) with auto-prune on context overflow |
| **Tool timeout** | 30s per tool (bash uses config timeout), returns error with hint instead of hanging |

## Hybrid Local + Cloud Routing

Pawan supports hybrid routing — use a local model first (via MLX, Ollama, or llama.cpp/lancor), fall back to NIM cloud:

```toml
# pawan.toml
provider = "mlx"
model = "mlx-community/Qwen3.5-9B-4bit"
temperature = 0.6
max_tokens = 4096
max_tool_iterations = 20

[cloud]
provider = "nvidia"
model = "minimaxai/minimax-m2.5"
```

The local model runs at $0/token. If it's down (OOM, Mac asleep), pawan seamlessly falls back to NIM cloud. Zero manual intervention.

## MLX on Mac Mini M4 — Honest Assessment

We switched from llama.cpp GGUF to `mlx_lm.server` (Apple Silicon native) for local inference. Here's what we actually observed:

### MLX vs llama.cpp

| | mlx_lm.server | llama.cpp |
|---|---|---|
| **Hardware** | Apple Silicon (Metal GPU) | Cross-platform (CPU + GPU) |
| **Format** | MLX native (safetensors) | GGUF |
| **Speed on M4 16GB** | ~40 tok/s (4-bit Qwen3.5-9B) | ~20 tok/s |
| **Memory** | Unified memory — efficient | Separate GPU/CPU split |
| **API** | OpenAI-compatible, localhost:8080 | OpenAI-compatible, localhost:8080 |

**Setup:** `uv tool install mlx-lm`, then `mlx_lm.server --model mlx-community/Qwen3.5-9B-4bit`. Persisted via launchd plist on Mac, exposed to VPS via SSH tunnel.

### What MLX handles well

- Targeted edits: "add a test for this function", "fix this compiler error"
- Simple implementations: bloom filter, fenwick tree, trie — wrote correct code on first try
- Tool calls: individual tool call format is reliable (~85% success rate)

### Where MLX falls short

- **Timeout on complex tasks**: At 5–10s/inference call, a 300s budget is exhausted during the exploration phase (reading files, checking dirs) before any code gets written. Leftist heap and suffix array both timed out.
- **Algorithmic bugs**: When it does write complex code under time pressure, correctness suffers. The suffix array binary search had inverted comparison directions; the treap's split-based remove was conceptually wrong. Both compiled but failed tests.
- **Task-level completion**: ~60% for "implement this data structure from scratch". Individual tool calls are fine; it's sustained multi-step reasoning that breaks down.

### Mitigation

Front-load prompts with all context the model would otherwise explore: exact file path, existing structure, exact function signature. Skip exploration entirely. This gets task completion up to ~80% for moderate-complexity tasks.

## Dogfood Stats

Updated 2026-04-12:

- **12 NIM models** benchmarked via nimakai latency + real-world pawan task dogfooding
- **Fastest task completion**: Qwen3.5 122B (13.6s for healing module review)
- **Highest SWE-bench**: MiniMax M2.5 (80.2%)
- - **16 data structures** in grind workspace
- **208 TUI + CLI tests** passing, zero clippy warnings
- **34 tools** in 3 tiers (Core/Standard/Extended) with auto-install via mise
- **Multi-model thinking support**: Qwen (`enable_thinking`), Gemma (`enable_thinking`), GLM (`enable_thinking` + `clear_thinking`), Mistral Small 4 (`reasoning_effort`), DeepSeek (`thinking`)
- **Token budget tracking**: thinking vs action token split visible in TUI and CLI

## Running Your Own Triage

```bash
# Latency benchmark via nimakai
nimakai --once -m "qwen/qwen3.5-122b-a10b,minimaxai/minimax-m2.5,stepfun-ai/step-3.5-flash"

# Test a model with a real coding task
pawan task "read src/lib.rs and identify the top 3 issues"

# Override model for comparison
PAWAN_MODEL=minimaxai/minimax-m2.5 pawan task "read src/lib.rs and identify the top 3 issues"
```
