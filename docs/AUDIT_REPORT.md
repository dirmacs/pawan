# Pawan TUI Enhancement Project Audit Report

## Phases 1-4 Audit Results

### 1. Model Selector UI
- **Status**: Implemented
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 881: handler, Line 1602: render, Line 655: keys)
- **Details**:
    - `/model` (or `/m`) without arguments correctly opens a visual modal.
    - Supports real-time filtering as the user types.
    - Selection via Enter correctly updates the active model and notifies the agent task.
- **Gaps/Bugs**: None found.

### 2. Session Browser UI
- **Status**: Implemented
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 947: handler, Line 1661: render, Line 694: keys)
- **Details**:
    - `/sessions` opens a modal listing all saved sessions from `~/.pawan/sessions/`.
    - Shows session ID, model, and message count.
- **Gaps/Bugs**:
    - **[BUG] Messages Not Loaded**: While selecting a session correctly updates the model and status bar, it **fails to populate the `messages` list** in the TUI. The conversation remains empty or preserves previous state instead of showing the loaded history.

### 3. Auto-save Engine
- **Status**: Partial
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 537: periodic, Line 544: on exit, Line 1708: implementation)
- **Details**:
    - Periodic autosave (every 5 minutes) is implemented using a timer in the event loop.
    - A final autosave is triggered on exit.
- **Gaps/Bugs**:
    - **[BUG] Session Fragmentation**: The `App` state does not track a "current session ID". Consequently, every autosave operation creates a **completely new session file** with a new UUID prefix, rather than updating an existing one.
    - **[GAP] Tool Calls Lost**: The `autosave()` function converts `DisplayMessage` to `Message` but hardcodes `tool_calls: Vec::new()`, meaning all tool execution history is stripped from autosaved files.
    - **[GAP] Tags Ignored**: `autosave()` uses `Session::new()` which defaults to empty tags, ignoring any tags added via `/tag`.

### 4. Session Tagging
- **Status**: Implemented
- **Source File**: 
    - `crates/pawan-cli/src/tui/mod.rs` (Line 1086: handler)
    - `crates/pawan-core/src/agent/session.rs` (Line 66: logic)
- **Details**:
    - `/tag add`, `/tag rm`, `/tag list`, and `/tag clear` subcommands are fully implemented in the TUI.
    - Logic in `pawan-core` handles sanitization and prevents duplicates.
- **Gaps/Bugs**:
    - **[GAP] Visual Feedback**: Active tags for the session are not visible in the TUI header or status bar; they can only be viewed by running `/tag list`.
    - **[GAP] Persistence**: Since autosave is currently broken for tags, they are only persisted if a user manually runs `/save`.

## Phases 5-8 Audit Results

### 5. Session Export
- **Status**: Implemented
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 1003: `/export` command, Line 1163: implementation)
- **Details**:
    - Supports four formats: Markdown, HTML, JSON, and Plain Text.
    - HTML export is self-contained with embedded CSS for easy viewing.
    - JSON export includes metadata and a simplified message list.
- **Gaps/Bugs**:
    - **[BUG] JSON Format Inconsistency**: `export_as_json` uses `format!("{:?}", msg.role)` which produces capitalized roles (e.g., "User"), while the rest of the system expects lowercase (e.g., "user"). This prevents exported JSON from being easily re-imported or processed by standard tools.
    - **[GAP] Lossy Export**: JSON export only includes tool call names and success status, omitting arguments and results. This makes it less useful for debugging compared to the internal session storage.

### 6. Session Pruning
- **Status**: Implemented
- **Source File**: 
    - `crates/pawan-core/src/agent/session.rs` (Line 434: `prune_sessions` logic)
    - `crates/pawan-cli/src/tui/mod.rs` (Line 1060: `/prune` command)
- **Details**:
    - Supports retention policies based on max age (days) and max session count.
    - Correctly skips sessions with protected tags (if specified in the policy).
- **Gaps/Bugs**:
    - **[GAP] CLI Limitation**: The `/prune` command hardcodes an empty `keep_tags` vector, meaning users cannot currently protect tagged sessions from the CLI.
    - **[GAP] Fragile Parsing**: The argument parser for `/prune` is strict (e.g., requires `7d` or `10s`); it does not handle spaces between numbers and units.

### 7. Tool Idle Timeout
- **Status**: Partial (Config Only)
- **Source File**: 
    - `crates/pawan-core/src/config/mod.rs` (Line 85: `tool_call_idle_timeout_secs`)
    - `crates/pawan-core/src/agent/mod.rs` (Line 189: `last_tool_call_time`)
- **Details**:
    - The configuration field exists with a default value of 300s (5 minutes).
    - `PawanAgent` correctly updates `last_tool_call_time` after every successful iteration of tool calls.
- **Gaps/Bugs**:
    - **[CRITICAL BUG] Missing Enforcement**: While the agent tracks the time of the last tool call, it **never checks it** against the timeout configuration. There is no background task or check in the main execution loop to interrupt a stalled session.
    - **[GAP] No Stalled Provider Recovery**: The requirement for "graceful recovery from stalled providers" (racing stream against cancel_event) is completely missing from the implementation.

### 8. Session Search
- **Status**: Implemented
- **Source File**:
    - `crates/pawan-core/src/agent/session.rs` (Line 400: `search_sessions` logic)
    - `crates/pawan-cli/src/tui/mod.rs` (Line 1039: `/ss` command)
- **Details**:
    - Implements full-text content matching across all saved sessions.
    - Results include hit counts, session metadata, and context previews for matching messages.
- **Gaps/Bugs**:
    - **[BUG] Messy CLI Code**: The `/ss` command handler in `tui/mod.rs` contains duplicated lines and broken indentation, suggesting it was hastily committed.
    - **[GAP] Performance**: Uses a linear scan of all JSON files on every search. While acceptable for dozens of sessions, it will degrade as the session history grows.

## Phases 9-13 Audit Results

### 9. Session Sharing (Import/Export flows)
- **Status**: Partial
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 1003: `/export`, Line 1187: implementation)
- **Details**:
    - Multiple export formats (Markdown, HTML, JSON, TXT) are fully implemented.
    - Exported files are correctly saved to the specified path with role-based formatting.
- **Gaps/Bugs**:
    - **[GAP] No Import**: There is no counterpart `/import` command or CLI tool to bring sessions back into the system from exported files.
    - **[GAP] No Sharing Protocol**: "Sharing" is limited to manual file transfer of JSON/MD files; no integrated sharing mechanism exists.

### 10. Typed Event Stream
- **Status**: Partial
- **Source File**: `crates/pawan-cli/src/tui/mod.rs` (Line 32: `AgentEvent` definition)
- **Details**:
    - `AgentEvent` enum successfully decouples the agent task from the TUI rendering loop.
    - Handles tokens, tool lifecycle (`ToolStart`, `ToolComplete`), and permission requests.
- **Gaps/Bugs**:
    - **[GAP] Event Granularity**: Events are still coarse. Many UI updates (like status changes or session metadata updates) are still handled through direct mutation of the `App` state rather than through a comprehensive, fully-typed event stream.

### 11. ToolCoordinator Adoption
- **Status**: Missing (Infrastructure Only)
- **Source File**: `crates/pawan-core/src/coordinator/mod.rs`
- **Details**:
    - A full `ToolCoordinator` implementation exists in `pawan-core`, featuring multi-turn orchestration, parallel execution, and token usage tracking.
- **Gaps/Bugs**:
    - **[GAP] Not Integrated**: `PawanAgent` (in `crates/pawan-core/src/agent/mod.rs`) does not yet use the `ToolCoordinator`. It continues to use its original, simpler tool-calling loop. This prevents the agent from benefiting from the coordinator's advanced features.

### 12. thulp-mcp Migration
- **Status**: Implemented
- **Source File**:
    - `crates/pawan-mcp/Cargo.toml` (Line 30)
    - `crates/pawan-mcp/src/server.rs` (Full file, 279 LOC)
- **Details**:
    - Successfully migrated from `rmcp` to `thulp-mcp` (v0.3.2).
    - The MCP server has been completely rewritten using the `thulp-mcp` builder pattern and `ToolHandler` trait.
- **Gaps/Bugs**: None found. The 279-LOC rewrite requirement from the status report appears satisfied.

### 13. Documentation (README, commands.md)
- **Status**: Implemented
- **Source File**:
    - `README.md`
    - `docs/content/commands.md`
- **Details**:
    - Documentation is comprehensive and generally reflects the implemented features.
    - `commands.md` accurately lists slash commands like `/tag`, `/ss`, `/export`, and `/handoff`.
- **Gaps/Bugs**:
    - **[GAP] Feature Drift**: `commands.md` lists some features (like `pawan tasks` and `pawan distill`) as fully featured, while their CLI implementation in `main.rs` may still be maturing.
    - **[GAP] Accuracy**: `/load` in TUI claims "Full message loading not yet implemented", but the documentation does not mention this limitation.
