+++
title = "Model Triage"
+++

Pawan is model-agnostic — it works with any OpenAI-compatible API. We've triaged models on NVIDIA NIM and local inference for real-world coding agent tasks: tool calling, multi-step reasoning, code generation, and self-healing.

## Triage Results (updated 2026-03-19)

Tested across 600+ cumulative tool calls, 9 autonomous data structure builds, and 318 tests across the workspace.

### Tier 1 — Production Ready

| Model | Provider | Tool Call Success | Notes |
|-------|----------|-------------------|-------|
| **StepFun Flash** | NIM | 98.9% | Best overall. Think-token interleaving adds reasoning depth. Primary cloud model. |
| **Qwen3.5-Coder-32B** | NIM | 97%+ | Excellent structured output. Fast. Great for batch tasks. |
| **Qwen3.5-9B 4-bit** (local) | MLX | ~85% tool calls | Runs on Mac Mini M4 via `mlx_lm.server`. $0 inference. Good for targeted edits; unreliable for complex algorithms from scratch (see below). |

### Tier 2 — Usable with Guardrails

| Model | Provider | Tool Call Success | Notes |
|-------|----------|-------------------|-------|
| **Kimi-K2** | NIM | ~90% | Good reasoning, occasional JSON formatting issues. |
| **Qwen3.5-122B** | NIM | ~93% | Excellent quality but slower. Better for complex single-shot tasks. |
| **Mistral Small 24B** | NIM | ~88% | Decent for simple tasks. Struggles with multi-step chains. |

### Tier 3 — Avoid for Agents

| Model | Provider | Issue |
|-------|----------|-------|
| **Devstral 2 123B** | NIM | Hangs on tool calls. Output truncation on multiline JSON args. |
| **DeepSeek V3.2** | NIM | Infinite loop on tool calling. Never terminates. |
| **GLM-4.7** | NIM | Accepts requests, never responds. Endpoint dead. |

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

From the 2026-03-18/19 grind sessions:

- **600+ tool calls** across all models
- **98.9% individual tool call success** (cloud, with guardrails)
- **~85% individual tool call success** (MLX local)
- **~60% full task completion** (MLX, complex algorithmic tasks)
- **9 data structures** built in grind workspace: bloom filter, fenwick tree, skip list, trie, segment tree, DSU, treap, suffix array, leftist heap
- **318 tests** passing: 245 pawan-core workspace + 73 grind
- **27 tools** exercised across file ops, git, search, native CLI wrappers
- **Hybrid routing** active: MLX local → StepFun cloud fallback

## Running Your Own Triage

```bash
# Test a model with a simple coding task
pawan run "create a Rust function that checks if a string is a palindrome" \
  --timeout 60 --verbose

# Compare local vs cloud
PAWAN_PROVIDER=mlx pawan run "implement a binary search tree" --output json
PAWAN_PROVIDER=nvidia PAWAN_MODEL=step-ai/step-2-flash pawan run "implement a binary search tree" --output json
```
