+++
title = "Model Triage"
+++

Pawan is model-agnostic — it works with any OpenAI-compatible API. We've triaged models on NVIDIA NIM and local inference for real-world coding agent tasks: tool calling, multi-step reasoning, code generation, and self-healing.

## Triage Results (2026-03-18)

Tested across 470+ cumulative tool calls, 32 autonomous data structure builds, and 164 test runs.

### Tier 1 — Production Ready

| Model | Provider | Tool Call Success | Notes |
|-------|----------|-------------------|-------|
| **StepFun Flash** | NIM | 98.9% | Best overall. Think-token interleaving adds reasoning depth. Primary model. |
| **Qwen3.5-Coder-32B** | NIM | 97%+ | Excellent structured output. Fast. Great for batch tasks. |
| **Qwen3.5-9B** (local) | llama.cpp | 95%+ | Runs on Mac Mini M4 (16GB). $0 inference. Perfect for dev/dogfooding. |

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

Pawan supports hybrid routing — try a local model first (via SSH tunnel, Ollama, or llama.cpp), fall back to NIM cloud:

```toml
# pawan.toml
provider = "openai"
model = "Qwen3.5-9B-Q4_K_M"

[cloud]
provider = "nvidia"
model = "step-ai/step-2-flash"
```

The local model runs at $0/token. If it's down (OOM, Mac asleep), pawan seamlessly falls back to NIM cloud. Zero manual intervention.

## MLX vs llama.cpp

Both run models locally on Mac, but they differ in performance and format:

| | mlx_lm.server | llama.cpp |
|---|---|---|
| **Hardware** | Apple Silicon (Metal GPU) | Cross-platform (CPU + GPU) |
| **Format** | MLX native (safetensors) | GGUF |
| **Speed on M4** | ~2x faster than llama.cpp | Baseline |
| **Memory** | Unified memory — efficient on M4 | Separate GPU/CPU split |
| **API** | OpenAI-compatible, localhost:8080 | OpenAI-compatible, localhost:8080 |
| **API key** | None needed | None needed |

**Recommendation:** prefer MLX on Mac M4. The unified memory architecture and Metal GPU give a significant throughput advantage, especially on 4–8 bit quantized models like `mlx-community/Qwen3.5-9B-4bit`.

## Dogfood Stats

From the 2026-03-18 marathon session:

- **470+ tool calls** across all models
- **98.9% success rate** (with guardrails)
- **32 data structures** built autonomously (fibonacci heap, threadpool, MPMC queue, FSM, etc.)
- **164 tests** written and passing
- **27 tools** exercised across file ops, git, search, native CLI wrappers
- **3 concurrent loops** (GRIND + HUSTLE + SWARM) running alternating pawan agents

## Running Your Own Triage

```bash
# Test a model with a simple coding task
pawan run "create a Rust function that checks if a string is a palindrome" \
  --timeout 60 --verbose

# Compare models
PAWAN_MODEL=step-ai/step-2-flash pawan run "implement a binary search tree" --output json
PAWAN_MODEL=qwen/qwen3.5-coder-32b-instruct pawan run "implement a binary search tree" --output json
```
