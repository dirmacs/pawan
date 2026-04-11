# Pawan rmcp usage inventory — 2026-04-11

Source: background research agent run from pawan main loop, scanning /opt/pawan
+ /opt/thulp/crates/thulp-mcp @ 0.3.1.
Tracks: pawan tasks #85, #86, #87, #88, #89 (Phase 11 rmcp-removal track).

## TL;DR

3 Rust files in /opt/pawan touch rmcp, all under `crates/pawan-mcp/src/`. Total
998 LOC. Heaviest by line count is `manager.rs`; deepest by API coupling is
`server.rs`. Two parity gaps in thulp-mcp 0.3.1 hard-block rmcp removal:
the entire server-side surface, and stdio-child-process env-var injection.

## File-by-file rmcp references

| File | LOC | rmcp symbols used |
|---|---:|---|
| `/opt/pawan/crates/pawan-mcp/src/manager.rs` | 371 | `rmcp::model::Tool`; `rmcp::service::{Peer, RoleClient, ServiceExt}`; `rmcp::transport::{ConfigureCommandExt, TokioChildProcess}`; `().serve(transport)`; `service.list_all_tools()`; `service.peer()`; `service.waiting()` |
| `/opt/pawan/crates/pawan-mcp/src/bridge.rs` | 357 | `rmcp::model::{CallToolRequestParam, Content, Tool, RawContent, RawTextContent, RawImageContent, Annotated, JsonObject}`; `rmcp::service::{Peer, RoleClient}`; `peer.call_tool(...)`; `Content.raw` pattern match |
| `/opt/pawan/crates/pawan-mcp/src/server.rs` | 270 | `rmcp::handler::server::tool::ToolRouter`; `rmcp::handler::server::wrapper::Parameters`; `rmcp::model::*` (CallToolResult, InitializeRequestParam, InitializeResult, ProtocolVersion, ServerCapabilities, ToolsCapability, Implementation); `rmcp::service::{RequestContext, RoleServer, ServiceExt}`; `rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler}`; `rmcp::transport::io::stdio()`; `server.serve(transport)`; `service.waiting()` |

Non-code mentions (don't block removal but need touchups):
- `/opt/pawan/Cargo.lock` (rmcp + rmcp-macros)
- `/opt/pawan/README.md:109`
- `/opt/pawan/crates/pawan-cli/README.md:16`
- `/opt/pawan/crates/pawan-mcp/Cargo.toml:23-28`
- `/opt/pawan/crates/pawan-mcp/README.md:7`
- `/opt/pawan/docs/content/architecture.md:110,114`
- `/opt/pawan/docs/img/architecture.svg:61`
- `/opt/pawan/docs/static/architecture.svg:61`
- `/opt/pawan/docs/public/architecture.svg:61`

Total Rust: 3 files, 998 LOC + 9 non-code touch points.

## rmcp API surface that pawan actually uses

Cargo features pawan enables: `client, server, transport-io, transport-child-process`.

### Client side (manager.rs + bridge.rs)
- `Peer<RoleClient>`, `().serve(transport)` from `ServiceExt`
- `service.list_all_tools()`, `service.peer().clone()`, `service.waiting()` keep-alive
- `Peer::call_tool(CallToolRequestParam{name, arguments})`
- `TokioChildProcess` + `ConfigureCommandExt` (env injection via `cmd.env(k,v)`)
- `rmcp::model::{Tool, Content, RawContent::{Text,Image}, RawTextContent, RawImageContent, Annotated, JsonObject}`

### Server side (server.rs)
- `ServerHandler` trait with async `initialize`
- `#[tool_router]` + `#[tool(name, description)]` attribute macros
- `ToolRouter<Self>` field; `Self::tool_router()`
- `Parameters<T: JsonSchema>` request wrapper
- `CallToolResult::{success, error}`; `Content::text`
- `InitializeRequestParam`; `InitializeResult`; `ProtocolVersion::V_2024_11_05`; `ServerCapabilities`; `ToolsCapability`; `Implementation`; `RequestContext<RoleServer>`; `context.peer.set_peer_info()`
- `ErrorData as McpError`
- `rmcp::transport::io::stdio()` (stdio server transport)
- `server.serve(transport)` + `service.waiting()`

## Behavioral parity vs thulp-mcp 0.3.1

| rmcp feature pawan uses | thulp-mcp 0.3.1 | Notes |
|---|---|---|
| `list_tools` (client) | ✅ | `McpClient::list_tools()` cached via `transport.rs` |
| `call_tool` (client)  | ✅ | `McpClient::call_tool(name, Value) -> ToolResult` |
| Stdio child-process client | ⚠️ partial | `McpTransport::new_stdio(name, command, args)` — **NO env injection API** |
| HTTP transport | ✅ (bonus) | Unused by pawan |
| Resources list/read/templates | ⚠️ stub | `ResourcesClient` serves only locally `register()`ed items; `read()` is placeholder |
| Resource subscribe | ⚠️ stub | In-memory `RwLock<Vec<String>>`, no JSON-RPC wire calls |
| Prompts list/get | ⚠️ stub | Serves only locally `register()`ed prompts |
| Server side (PawanServer) | ❌ MISSING | No `ServerHandler`, no `tool_router`/`#[tool]`, no `serve()`, no stdio server transport, no `initialize` |
| Typed RawContent enum | ⚠️ lossy | `ToolResult` wraps `Value`; `extract_text_content` path needs new shape |
| `result.is_error` flag | ❓ unclear | `ToolResult::success(Value)` suggests an error ctor exists; mapping needs porting |

Pawan does **not** currently consume MCP resources or prompts, so the
resources/prompts stubs do not block a pure-client migration. The server-side
gap and the env-var gap **do**.

## Migration estimate (full rmcp removal, assuming parity)

Total LOC in rmcp-importing files: 998.

Realistic diff:

| File | Effort |
|---|---|
| `manager.rs` | ~60 changed lines — rewrite `connect_one` around `McpClient::connect_stdio`; replace `Peer<RoleClient>` with `Arc<Mutex<McpClient>>`; add env-var shim |
| `bridge.rs`  | ~70 changed lines — swap `McpTool→ToolDefinition`, rewrite `extract_text_content` against `ToolResult`, replace `Annotated`/`RawContent` test fixtures (note: `bridge.rs` will be **deleted** in favor of `thulp_bridge.rs` which is already wired to the typed `ToolResult` form) |
| `server.rs`  | full **270-LOC rewrite** — `#[tool_router]`/`#[tool]` macro pattern has no thulp-mcp counterpart |
| `Cargo.toml` | −1 dep line, −4 comment lines, thulp-mcp version bump |
| Docs/lockfile/SVG | ~12 cosmetic edits across 7 non-code files |

**Ballpark churn: 500–700 LOC + ~50 LOC of replacement tests.**

## Hard blockers (must ship in thulp-mcp first)

| # | Blocker | Maps to task |
|---|---|---|
| 1 | **Server-side API.** `ServerHandler` equivalent, tool registration (macro or builder), `CallToolResult::{success,error}` analogues, `ServerCapabilities`/`Implementation` advertisement, stdio server transport. Without this, `pawan mcp serve` cannot migrate. | #87 |
| 2 | **Env var injection on stdio child-process client.** rs-utcp's `McpProvider::new_stdio(name, command, args)` has no env hook. pawan-mcp's `McpServerConfig.env` (used by all production MCP servers pawan spawns) has nowhere to land. Need a `connect_stdio_with_env` constructor or a command-wrapping shim. | #88 |
| 3 | **`is_error` flag on `ToolResult`.** `bridge.rs` uses `result.is_error.unwrap_or(false)`. Need an explicit error path that doesn't conflate "tool ran but reported an error" with "transport-level failure". | #87 (or new) |
| 4 | **Typed content access.** Pawan distinguishes `RawContent::Text` vs `::Image`. `Value`-based `ToolResult` needs to expose text fragments cleanly. | #87 (or new) |
| 5 | Real resources/* and prompts/* wire calls — not blocking today (unused), nice to have. | #85, #86 |

**Blockers 1 and 2 are the hard gates.** 3 and 4 can be folded into #87.

## Verification commands the agent ran

```bash
deagle rg "rmcp" /opt/pawan
deagle rg "ServerHandler|tool_router|handler::server|serve\(" /opt/thulp/crates/thulp-mcp/
wc -l /opt/pawan/crates/pawan-mcp/src/{manager,bridge,server}.rs
```
