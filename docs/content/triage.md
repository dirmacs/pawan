+++
title = "Model Triage"
+++

Pawan is model-agnostic — it works with any OpenAI-compatible API. We've triaged models on NVIDIA NIM and local inference for real-world coding agent tasks: tool calling, multi-step reasoning, code generation, and self-healing.

## Triage Results (updated 2026-03-19)

Tested across 1000+ cumulative tool calls, 16 data structure builds, and 372 tests across the workspace. 11 NIM models benchmarked on the B-Tree ringer test (2026-03-21).

### Tier 1 — Production Ready

| Model | Provider | Tool Call | Coding | Notes |
|-------|----------|----------|--------|-------|
| **Mistral Small 4 119B** | NIM | Good | **Best** | First 100% autonomous score (interval tree 6/6). Self-corrects via semantic reasoning. Primary model. |
| **StepFun Flash** | NIM | **Best** (98.9%) | Good | Best for multi-step orchestration. Cloud fallback. |
| **MiniMax M2.5** | NIM | Good | Good (4/5 B-Tree) | Tied with Mistral on B-Tree. Fewer tool calls needed. |
| **Qwen3.5-9B-OptiQ-4bit** (local) | MLX | ~85% | Execution only | 17-18 tok/s, $0. `enable_thinking: false` required. Can't generate complex algos. |

### Tier 2 — Usable with Guardrails

| Model | Provider | Notes |
|-------|----------|-------|
| **Qwen3.5-122B** | NIM | Strong code but can't fix borrow checker in time (6 errors on B-Tree). |
| **GPT-OSS 120B** | NIM | Fixes things but breaks other things. Destructive fix loops. |
| **Nemotron-Super-49B** | NIM | Too simple implementations (1/5 B-Tree). |

### Tier 3 — Avoid for Agents

| Model | Provider | Issue |
|-------|----------|-------|
| **DeepSeek V3.2** | NIM | Infinite loop on tool calling. Never terminates. |
| **Mistral-Nemotron** | NIM | Describes actions but never makes tool calls. |
| **Nemotron-3-Nano-30B** | NIM | All thinking tokens, zero tool calls. |
| **Nemotron-Ultra-253B** | NIM | Hit max iterations, never completed. |
| **Nemotron-3-Super-120B** | NIM | Missing Default trait, compile errors. |
| **Nemotron-3-Nano-4B** | MLX | Broken chat template, garbled output. |
| **Nemotron-Cascade-8B** | MLX | Can't disable thinking. Burns all tokens. |

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

Pawan supports hybrid routing — use a local model first (via MLX, Ollama, or llama.cpp), fall back to NIM cloud:

```toml
# pawan.toml
provider = "mlx"
model = "mlx-community/Qwen3.5-9B-4bit"
temperature = 0.6
max_tokens = 4096
max_tool_iterations = 20

[cloud]
provider = "nvidia"
model = "step-ai/step-2-flash"
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

Updated 2026-03-21:

- **1000+ tool calls** across 11 models
- **11 NIM models** benchmarked on B-Tree ringer test
- **First 100% autonomous score**: Mistral Small 4 wrote interval tree (6/6 tests)
- **Best coding accuracy**: Mistral Small 4 (4/5 B-Tree, 6/6 interval tree, self-refactored 9 callsites)
- **Best tool calling**: StepFun Flash (98.9% success rate)
- **16 data structures** in grind workspace: bloom filter, fenwick, skip list, trie, segment tree, DSU, treap, suffix array, leftist heap, radix tree, pairing heap, splay tree, rope, AVL tree, LRU cache, interval tree
- **119 grind tests + 207 pawan-core tests + 46 TUI tests = 372 total**
- **29 tools** in 3 tiers (Core/Standard/Extended) with auto-install via mise
- **Pawan dogfoods itself**: wrote 14 tests for its own git.rs, native.rs, bash.rs, agent.rs
- **Token budget tracking**: thinking vs action token split visible in TUI and CLI

## Running Your Own Triage

```bash
# Test a model with a simple coding task
pawan run "create a Rust function that checks if a string is a palindrome" \
  --timeout 60 --verbose

# Compare local vs cloud
PAWAN_PROVIDER=mlx pawan run "implement a binary search tree" --output json
PAWAN_PROVIDER=nvidia PAWAN_MODEL=step-ai/step-2-flash pawan run "implement a binary search tree" --output json
```
