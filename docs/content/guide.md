+++
title = "Getting Started"
+++

## Architecture

![Pawan Architecture](/pawan/architecture.svg)

## Install

```bash
cargo install pawan
```

Or build from source:

```bash
git clone https://github.com/dirmacs/pawan && cd pawan
cargo install --path crates/pawan-cli
```

## Setup

Set your NVIDIA NIM API key (free tier available at [build.nvidia.com](https://build.nvidia.com)):

```bash
export NVIDIA_API_KEY=nvapi-...
```

Or create a `.env` file in your project:

```
NVIDIA_API_KEY=nvapi-...
```

### Local Inference (Free, Optional)

The `mlx` provider runs a model locally on Mac via [mlx_lm.server](https://github.com/ml-explore/mlx-examples/tree/main/llms). No API key needed, $0/token. Optional — pawan defaults to Mistral Small 4 on NVIDIA NIM.

```toml
provider = "mlx"
model = "mlx-community/Qwen3.5-9B-OptiQ-4bit"
base_url = "http://localhost:8080/v1"
```

Verify your setup:

```bash
pawan doctor
```

## Initialize a project

```bash
cd your-project
pawan init
```

This creates:
- `pawan.toml` — configuration file
- `PAWAN.md` — project context (pawan reads this to understand your codebase)
- `.pawan/` — local pawan directory

## First run

```bash
# Interactive TUI
pawan

# Or try a quick task
pawan explain src/main.rs
```

## Configuration

Priority: CLI flags > environment variables > pawan.toml > defaults

### Environment variables

| Variable | Description |
|----------|-------------|
| `PAWAN_MODEL` | Model override (e.g., `qwen/qwen3.5-397b-a17b`) |
| `PAWAN_PROVIDER` | Provider: `nvidia`, `ollama`, `openai`, `mlx` |
| `PAWAN_TEMPERATURE` | Temperature (0.0-2.0) |
| `PAWAN_MAX_TOKENS` | Max output tokens |
| `PAWAN_MAX_ITERATIONS` | Max tool-calling iterations |

### pawan.toml

```toml
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

## Common workflows

### Fix a broken build

```bash
pawan heal
```

### AI-powered commit

```bash
# Stage all, generate message, confirm, commit
pawan commit -a

# Just preview the message
pawan commit --dry-run

# Skip confirmation
pawan commit -a -y
```

### Code review

```bash
# Review all changes
pawan review

# Review only staged changes
pawan review --staged
```

### Continuous healing

```bash
# Check every 10 seconds, auto-commit fixes
pawan watch --interval 10 --commit
```

### Headless scripting

```bash
# Single prompt
pawan run "add error handling to the config parser"

# From file
pawan run -f task.md --timeout 300 --output json
```
