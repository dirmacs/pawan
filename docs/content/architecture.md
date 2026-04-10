+++
title = "Architecture"
+++

## The Vibe Coding Loop

Pawan's core value proposition is the **vibe coding loop**: you describe
what you want in natural language, and pawan iterates on the implementation
using the Rust compiler (and clippy, and tests) as the source of truth. The
compiler becomes your auditor — every iteration either compiles cleanly or
surfaces a diagnostic that the LLM reads and fixes on the next turn.

```
  ┌─────────────┐   prompt    ┌───────────────┐
  │   User      │────────────▶│  PawanAgent   │
  └─────────────┘             └───────┬───────┘
                                      │
                             execute_with_all_callbacks
                                      │
            ┌─────────────────────────▼─────────────────────────┐
            │              agent/mod.rs loop                     │
            │  1. inject_core_memory  (eruka_bridge)             │
            │  2. send to LLM backend (NIM / ollama / ares)      │
            │  3. parse tool calls                               │
            │  4. execute tools (ToolRegistry)                   │
            │  5. feed results back                              │
            │  6. repeat until model stops calling tools         │
            │  7. archive_session + sync_turn  (eruka_bridge)    │
            └───────────────────────────────────────────────────┘
```

Source of truth:
- Agent loop: `crates/pawan-core/src/agent/mod.rs`
- Tool dispatch: `crates/pawan-core/src/tools/mod.rs`
- LLM backends: `crates/pawan-core/src/agent/backend/`
- Heal loop: `crates/pawan-core/src/healing/mod.rs`

## The Heal Loop (Compile-as-Auditor)

When you run `pawan heal` (or the model voluntarily triggers it during a
task), pawan runs `cargo check`, `cargo clippy`, and `cargo test`, parses
the output, and feeds structured diagnostics back to the LLM. The LLM edits
the code, pawan re-runs the checks, and the loop repeats until the build
is clean.

```
  ┌──────────────┐
  │ cargo check  │────▶ parse_diagnostics ────┐
  └──────────────┘                             │
  ┌──────────────┐                             ▼
  │ cargo clippy │────▶ parse_diagnostics ──▶ LLM ──▶ edit_file ──┐
  └──────────────┘                             ▲                    │
  ┌──────────────┐                             │                    │
  │ cargo test   │────▶ parse_test_output ────┘                    │
  └──────────────┘                                                   │
         ▲                                                           │
         └───────────────────────────────────────────────────────────┘
                             iterate until clean
```

The parser handles both rustc's JSON output (`--message-format=json`) and
its human-readable text fallback, which is critical when tests fail because
their output is mixed text. See `healing/mod.rs` for the dual-format parser
and its 26 regression tests.

## Integration Points

Pawan is the coding agent at the center of the **dirmacs stack**. Each
integration is opt-in and degrades gracefully when missing:

| Component | Crate / Path | Purpose | Graceful Fallback |
|-----------|--------------|---------|-------------------|
| **ares-server** | `agent/backend/ares_backend.rs` | LLM proxy with routing, NIM/Groq/Anthropic fan-out | OpenAI-compat backend |
| **eruka** | `eruka_bridge.rs` | Context memory (core + archival) | Short-circuit when disabled |
| **thulp-skills** | `tools/agent.rs` (skill loader) | SKILL.md discovery + execution | Uses built-in prompts |
| **thulpoff** | `skill_distillation.rs` | Refine skills via eval loop | Skill stays as-is |
| **deagle** | `tools/deagle.rs` | Code intelligence (graph + AST search) | Falls back to ripgrep |
| **daedra (MCP)** | `pawan-mcp` | Web search + external tools | Skipped if unreachable |
| **eruka-mcp** | Auto-discovered | 17 context tools | Uses direct eruka REST |

The eruka_bridge exposes 5 lifecycle/caching/export methods that the agent
loop calls directly without going through MCP — `sync_turn`,
`on_pre_compress`, `prefetch`, `get_context_cached`, `export_context` —
so turn lifecycle is fast even when MCP is off.

## Tool Dispatch Flow

```
  LLM response ──▶ parse tool_calls ──▶ ToolRegistry::get(name)
                                              │
                                              ▼
                                  ┌───────────────────────┐
                                  │     Tool trait        │
                                  │  - name()             │
                                  │  - description()      │
                                  │  - parameters_schema()│
                                  │  - execute(args)      │
                                  └──────────┬────────────┘
                                              │
         ┌────────────────┬───────────────────┼───────────────────┬────────────┐
         ▼                ▼                   ▼                   ▼            ▼
    ReadFileTool    BashTool (validated)  RipgrepTool          DeagleTool  McpToolBridge
                    (read-only cache,                          (subprocess) (namespaced)
                     compound check)
```

Every pawan tool implements the same `Tool` trait. External MCP tools are
wrapped in `McpToolBridge` which delegates to an rmcp `Peer<RoleClient>`.
This means the agent loop treats "read_file" and "mcp_daedra_web_search"
identically — the namespacing happens at the bridge layer.

## Safety Layers

Pawan has two permission models layered on top of tool execution:

1. **Blocklist at validation**: `validate_bash_command()` rejects
   destructive commands outright (`rm -rf /`, `mkfs`, `dd if=/dev/zero`,
   curl/wget piped to sh/bash/sudo, fork bombs, chmod -R 777 /).
2. **Read-only auto-allow**: `is_read_only()` recognizes side-effect-free
   commands and auto-approves them under Prompt permission — but it now
   splits compound operators (`&&`, `||`, `;`, `|`) and requires EVERY
   sub-command to be individually read-only. This was a SECURITY fix
   (task #70): `ls && rm file.txt` is no longer auto-approved.

File writes go through `validate_file_write()` which blocks:
- `.git/` path components (not `.gitignore` or `.github/`)
- credential files: `.env*`, `id_rsa`, `id_ed25519`, `credentials.json`
- system directories: `/etc/`, `/usr/`, `/bin/`, `/sbin/`, `/boot/`
- lock files trigger a warn log but are allowed

See `tools/bash.rs` and `tools/file.rs` for the full validation rules with
32 + 20 regression tests respectively.

## Why Rust for Vibe Coding

The vibe-coding loop works best when the auditor is fast, precise, and
deterministic. Rust's compiler hits all three:

- **Fast**: incremental `cargo check` on a changed file is usually under
  500ms — the LLM gets feedback before losing context.
- **Precise**: every error has a file:line:column, a code (E0xxx), and
  often a suggested fix. The JSON diagnostic format is machine-parseable.
- **Deterministic**: the same code always produces the same diagnostics.
  No flakiness, no environment drift.

This is the opposite of coding-in-a-dynamic-language where you have to
run the code to see if it's wrong. Pawan's heal loop amplifies this — it
can recover from 5-10 compiler errors per iteration, and because each
recovery is type-checked, you don't get degradation over long chains.

See the [Nu→Rust rewrite pattern memory][nu-rust] for a validated example:
`dirmacs-git-sync` was rewritten from a 538-LOC nushell script to a 1100-LOC
Rust crate with 157 tests, catching 4 classes of bugs at compile time that
the nushell version had hit in production.

[nu-rust]: https://github.com/dirmacs/pawan
