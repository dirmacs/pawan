# Ares ToolCoordinator — adoption map for pawan Phase 8 (#17)

Source: background research agent run from pawan main loop, scanning /opt/ares snapshot 2026-04-11.
Tracks: pawan task #17.

## Verdict

**Direct reuse of `ares::llm::ToolCoordinator` from `pawan-core` is NOT feasible.**

Recommended path: **Option B (port now) → Option A (extract subcrate later).**

## Why direct reuse fails

`ares` is a single flat workspace. `ToolCoordinator` lives in the root
`ares-server` crate at `/opt/ares/src/llm/coordinator.rs:247`. Importing it
forces the consumer to take on the entire `ares-server` dep graph (axum,
tower, jsonwebtoken, parking_lot, notify, sqlx, ~200 transitive crates),
because three unavoidable imports inside `coordinator.rs` chain into web
infrastructure:

| Import | Pulls in |
|---|---|
| `crate::types::{Result, ToolCall}` | `AppError` which `impl axum::response::IntoResponse` — drags axum + the web stack |
| `crate::llm::client::LLMClient` | Module imports `crate::utils::toml_config::{ModelConfig, ProviderConfig}` — drags the 1300-LOC `toml_config.rs` (hot reload, full server config) |
| `crate::tools::registry::ToolRegistry` | Imports `crate::utils::toml_config::{AresConfig, ToolConfig}` — same `toml_config.rs` again |

Pawan-core's charter (zero dirmacs deps in the lib crate) is incompatible with
this footprint.

## What's good about the ares side

- `ToolCoordinator` itself is **stateless** — zero coupling to auth, sessions,
  eruka, MCP, telemetry. Pure orchestrator over `(LLMClient, ToolRegistry, ToolCallingConfig)`.
- The `Tool` trait at `/opt/ares/src/tools/registry.rs:13` is **shape-identical**
  to pawan's at `/opt/pawan/crates/pawan/src/tools/mod.rs`: same four methods,
  same `Send + Sync`, same `Arc<dyn Tool>` storage. Differs only in error type.
- **Internal usage in ares = zero.** Searched all of `src/` and `tests/`:
  `ToolCoordinator` is defined and re-exported but never *constructed* anywhere
  in ares. Pawan would be its first real consumer.

## What's missing in the ares coordinator (gaps to close in pawan's port)

- **No streaming.** Always accumulates and returns once. Pawan's TUI needs streaming.
- **No cancellation.** No `CancellationToken`, no stop channel. Only stop is per-tool timeout + iteration cap.
- **No `execute_with_history`.** Always starts fresh from `(system, prompt)`.
- **No progress callback.** No way to surface "tool X started/finished" events to a TUI.
- **No mid-flight error recovery.** Failures get stuffed into `ToolCallRecord` with `success: false`.
- `ToolCallingConfig::include_tool_results` — **dead field**, never read by `execute()`. Drop it during the port.

## Public API surface (what to port verbatim)

`coordinator.rs` is 588 lines, ~100 of which are unit tests, ~100 of which are real logic.

### `pub struct ToolCoordinator`
```rust
pub struct ToolCoordinator {
    client: Box<dyn LLMClient>,
    registry: Arc<ToolRegistry>,
    config: ToolCallingConfig,
}
```

| Method | Signature |
|---|---|
| `new` | `(client, registry, config) -> Self` |
| `with_defaults` | `(client, registry) -> Self` |
| `execute` | `async (&self, system: Option<&str>, prompt: &str) -> Result<CoordinatorResult>` |
| `client` / `registry` / `config` | trivial accessors |

Internal helpers: `execute_tool_calls`, `execute_parallel`, `execute_sequential`, `execute_single_tool`.

### `pub struct CoordinatorResult`
```rust
pub struct CoordinatorResult {
    pub content: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub iterations: usize,
    pub finish_reason: FinishReason,
    pub total_usage: TokenUsage,
    pub message_history: Vec<ConversationMessage>,
}
```

### Supporting types (all `pub`)
- `ToolCallRecord { id, name, arguments, result, success, duration_ms, error }` — `Serialize + Deserialize`. Keep this.
- `FinishReason { Stop, MaxIterations, Error(String), UnknownTool(String) }`
- `ConversationMessage { role, content, tool_calls, tool_call_id }` + `MessageRole { System, User, Assistant, Tool }`
- `ToolCallingConfig { max_iterations: 10, parallel_execution: true, tool_timeout: 30s, stop_on_error: false }` — drop the dead `include_tool_results` field

## Recommended port plan (Option B)

Phase 8 deliverable: `pawan_core::coordinator` module that mirrors ares's
shape with the same struct names. Concrete steps:

1. New file `crates/pawan-core/src/coordinator/mod.rs`. Copy `coordinator.rs`
   verbatim from ares. Replace the three `crate::*` imports with pawan-local types.
2. Keep struct names identical (`ToolCoordinator`, `ToolCallingConfig`,
   `CoordinatorResult`, `ToolCallRecord`, `FinishReason`, `ConversationMessage`,
   `MessageRole`) so a future Option A swap is purely an import-path refactor.
3. Define a **1-method** `LLMClient` trait inside pawan-core (only the
   `generate_with_tools_and_history`-equivalent + `model_name()`). Do NOT port
   the 9-method ares trait — those extra methods are unused by the coordinator
   and only exist to serve ares's web layer.
4. Add `cancellation: CancellationToken` to `execute()` — pawan needs it now,
   ares can adopt it later when the subcrate gets extracted.
5. Add `tool_progress: Option<Box<dyn Fn(&ToolEvent) + Send + Sync>>` callback
   on `ToolCoordinator` for TUI streaming. Same reasoning.
6. Drop `ToolCallingConfig::include_tool_results` (dead field in ares).
7. Wire it into pawan's existing `PawanAgent::run` as an opt-in alternative to
   the current handcrafted tool loop. Feature-flag for one release, then
   promote to default.
8. Tests: lift ares's coordinator unit tests, port them onto pawan's tools.

Estimated diff: ~150 LOC of new pawan-core source + ~80 LOC of tests. Single
PR. No new deps beyond `tokio_util` (for `CancellationToken`) which pawan
already pulls in transitively.

### Option A (later, after B ships)

Once pawan-core has its own coordinator with the same struct names, file a
follow-up to extract `crates/ares-tool-core` from `/opt/ares`:

- Move `Tool`, `ToolRegistry`, `ToolCoordinator`, `ToolCallingConfig`,
  `CoordinatorResult`, `ToolCallRecord`, `FinishReason`, `ConversationMessage`,
  `MessageRole` into the new subcrate.
- Define a 1-method `LLMClient` trait there (same shape as pawan's port).
- Define a lightweight error enum (no axum, no `IntoResponse`).
- Untangle `LLMClient` from `toml_config` by moving `Provider::from_env` up
  into `ares-server`.
- Drop the `ToolConfig` dependency in `ToolRegistry`; accept a generic enable
  filter instead.
- ares-server re-exports from the subcrate.
- pawan-core swaps its local copy for `ares-tool-core` — pure import-path change.

This is a multi-PR ares refactor, but mechanical once pawan's port is in place.

## Specific gotchas to handle in the port

| Gotcha | Action |
|---|---|
| `AppError: IntoResponse` web coupling | Use pawan's existing `PawanError` |
| `LLMClient` 9-method trait pulls in `toml_config` | Define a 1-method local trait |
| `ToolRegistry` carries `HashMap<String, ToolConfig>` | Pawan already has its own `ToolRegistry`; reuse it |
| Internal usage in ares = 0 | Don't worry about staying in lockstep — pawan can lead |
| `include_tool_results` is dead | Drop it. Don't carry the bug across. |
| No streaming, cancellation, progress | Add all three in pawan's port. ares can pick them up later. |

## File reference (absolute paths in /opt/ares)

- `/opt/ares/src/llm/coordinator.rs` — canonical source to port from
- `/opt/ares/src/llm/client.rs` — `LLMClient` trait + `Provider` enum + `LLMResponse` + `TokenUsage`
- `/opt/ares/src/llm/mod.rs` — re-exports
- `/opt/ares/src/tools/registry.rs` — `Tool` trait + `ToolRegistry`
- `/opt/ares/src/types/mod.rs` — `ToolCall`, `AppError` (the axum-coupling problem at lines 619-647)
- `/opt/ares/src/utils/toml_config.rs` — `ToolConfig` (line 338) + `AresConfig` (line 25)
- `/opt/ares/Cargo.toml` — workspace + feature flags. Pattern reference: `mcp = ["dep:rmcp"]` was decoupled from postgres in 0.7.5; same trick applies to extracting `ares-tool-core`.
