# Intel: kon Research Report
**Date:** 2026-04-12
**Source:** https://github.com/0xku/kon

---

## What is kon

Kon is a lightweight Python 3.12+ terminal coding agent emphasizing minimal overhead: a sub-270-token core system prompt, six fixed built-in tools (`read`, `edit`, `write`, `bash`, `grep`, `find`), optional `web_search`/`web_fetch`, and TOML config at `~/.kon/config.toml`. Sessions are stored as append-only JSONL files. The TUI (Textual-based) supports `@`-fuzzy file references, Tab path completion, a command-queue of up to 5 prompts, and built-in theme selection (gruvbox, one-dark variants). Context loads from `AGENTS.md`/`CLAUDE.md` and skill files in `.kon/skills/`. Kon is not a Rust project — all patterns require translation, not direct porting.

---

## Architecture Decisions Worth Studying

- **Typed event stream** (`events.py`): every phase of a turn emits a typed dataclass event (`TurnStartEvent`, `ThinkingDeltaEvent`, `ToolApprovalEvent`, `TurnEndEvent`, etc.), decoupling the agent loop from UI rendering entirely.
- **Config versioned migration chain** (`config.py`): stepped `v0→v4` migrations with `_migrate_v0_to_v1` ... `_migrate_v3_to_v4`, atomic TOML writes via `tempfile`+`os.replace`, and timestamped `.bak` backups before migration.
- **Compaction with structured prompt** (`core/compaction.py`): `SUMMARIZATION_PROMPT` forces a Goal/Instructions/Discoveries/Accomplished/Files template, yielding resumable context for the next agent.
- **Handoff command** (`core/handoff.py`): `/handoff` spawns a focused new session using `HANDOFF_PROMPT_TEMPLATE` — strips noise, preserves file paths and constraints, outputs a ready-to-send user message.
- **Tool idle timeout with stream racing** (`turn.py`): `asyncio.wait(FIRST_COMPLETED)` races stream chunks against `cancel_event` and a configurable `tool_call_idle_timeout_seconds`, recovering gracefully from stalled providers.
- **Permission model on tools** (`tools/base.py`): `mutating: bool` field auto-approves read-only tools; mutating tools yield a `ToolApprovalEvent` and await user confirmation before executing.

---

## Specific Patterns to Port to Pawan

**Top priority:**

1. **Typed event stream** — Replace ad-hoc TUI callbacks with a sealed `AgentEvent` enum in `crates/pawan-core/src/agent/mod.rs`. All rendering in `crates/pawan-cli/src/tui/mod.rs` subscribes to a channel; loop logic emits events.
2. **Config versioned migration** — Add `config_version: u32` to `PawanConfig` in `crates/pawan-core/src/config/mod.rs`, implement `migrate_v0_to_v1` chain, atomic write with `.bak.{timestamp}` backup on upgrade.
3. **Structured compaction prompt** — Adopt kon's Goal/Instructions/Discoveries template in `crates/pawan-core/src/agent/session.rs` for context-window overflow handling.

**Medium priority:**

4. **Handoff command** — Add `/handoff` as a TUI slash command (`crates/pawan-cli/src/tui/mod.rs`) invoking a `generate_handoff_prompt()` call in `crates/pawan-core/src/agent/mod.rs`.
5. **Tool idle timeout** — Wire `tool_call_idle_timeout_seconds` config into the provider stream loop in `crates/pawan-core/src/agent/backend/`.
6. **`mutating` flag on Tool trait** — Add `fn mutating(&self) -> bool` to the `Tool` trait in `crates/pawan-core/src/tools/mod.rs`; auto-approve when `false`.

**Low priority:**

7. **`@`-fuzzy file references in prompt input** — Extend `crates/pawan-cli/src/tui/mod.rs` input widget with `@`-triggered completion.
8. **Command queue (up to N prompts)** — Buffer queued user messages in `crates/pawan-core/src/coordinator/mod.rs` for autonomous multi-step dispatch.
9. **HTML session export** — Add `/export` to session commands, producing a self-contained HTML transcript from the JSONL session file.

---

## Things That Validate Pawan's Current Design

- Pawan's `Tool` trait + `ToolRegistry` in `tools/mod.rs` mirrors kon's `BaseTool` ABC exactly — the right abstraction.
- Pawan's `skills.rs` with SKILL.md frontmatter discovery matches kon's `context/skills.py` — same convention, already implemented.
- TOML config via `pawan.toml` and JSONL session storage in `agent/session.rs` are confirmed best-practice by kon.
- Using ripgrep/fd as external search backends (pawan's `native.rs`) is the same approach kon takes.

---

## Things to Explicitly NOT Copy

- **Auto-download of fd/rg binaries** (`tools_manager.py`): pawan's `native.rs` already handles binary detection differently; pulling in aiohttp-style download logic adds complexity with no gain.
- **Python ContextVar config caching**: Rust's `OnceLock` or `Arc<RwLock<Config>>` is idiomatic; don't emulate Python's thread-local pattern.
- **Pydantic for tool param validation**: pawan uses serde + typed structs, which is strictly safer and faster — no regression here.
- **Textual TUI framework**: pawan is committed to ratatui; kon's widget/block decomposition is instructive for layout ideas but not portable.
