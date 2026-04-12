# GDevelop Research — 2026-04-12

Task #68: Quick relevance scan for 4ian/GDevelop.

## What it is

GDevelop (https://github.com/4ian/GDevelop) — open-source 2D/3D/multiplayer game engine.
- ~8K stars, active community
- Primary users: non-programmers, indie game devs
- Stack: C++ engine core, JS/TS frontend, React-based editor
- Visual "event" scripting system (block-based, like Scratch for games)
- Cross-platform: web, desktop, iOS, Android, Steam

## AI Integration (the only potential angle)

GDevelop has a built-in "GDevelop AI" feature (gpt-4o calls via their server) that:
- Generates behavior code snippets from natural language descriptions
- Creates game object configs from text prompts
- Operates via their proprietary cloud API — no local inference, no OpenAI-compat protocol

This is end-user AI (click a button in the editor), not agent-authoring infrastructure.

## Verdict: OUT OF SCOPE

Zero tie to dirmacs stack:
- Not Rust
- No MCP / OpenAI tool protocol
- No LLM orchestration primitives (no tool calls, no streaming, no agent loops)
- No CLI surface pawan could wrap
- No integration with ARES, eruka, thulp, or any dirmacs component

No surprising agent-authoring angle found. Closing task.
