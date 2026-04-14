# Pawan TUI Improvements - Implementation Roadmap

## Executive Summary

This roadmap outlines enhancements to pawan's Terminal User Interface including session persistence integration and a model selector feature, building upon pawan's existing session infrastructure while taking inspiration from oh-my-pi's UX patterns.

---

## Current State Analysis

### What Already Exists ✅

- **Session Management**: pawan-core has full session persistence with `Session` and `SessionSummary` structs
  - Auto-generated 8-character IDs (UUID v4 prefix)
  - Automatic timestamps (RFC3339 format)
  - Token usage tracking (total/completion/reasoning/action splits)
  - Session listing with model names and timestamps
  - File-based storage in `~/.pawan/sessions/`
  
- **CLI Commands**:
  - `pawan sessions` — lists saved sessions
  - `pawan chat --resume <id>` — resumes a session
  - `pawan run --save` — saves session after execution
  - `pawan distill --session <id>` — exports session to skill

- **TUI Foundations**:
  - Slash command framework (`/` prefix)
  - Status bar with operational state
  - Message routing and rendering system
  - Input handling with TextArea widget
  - Panel-based navigation (Input/Messages focus)
  - Activity panel for tool execution details
  
- **Agent Commands**:
  - `AgentCommand::SwitchModel` already defined in tui module
  - Model changes trigger via `/model <name>` slash command

### Key Code Structure Points

**App State (current)**:
```rust
struct App<'a> {
    model_name: String,
    messages: Vec<DisplayMessage>,
    session_tool_calls: u32,
    session_files_edited: u32,
    total_tokens: u64,
    // ... other state
    cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    event_rx: mpsc::UnboundedReceiver<AgentEvent>,
}
```

**Session Model**:
```
Session {
    id: String,        // "abcdef12" (8 chars)
    model: String,
    created_at: String, // RFC3339
    updated_at: String, // RFC3339
    messages: Vec<Message>,
    total_tokens: u64,
    iteration_count: u32,
}

SessionSummary {
    id: String,
    model: String,
    created_at: String,
    updated_at: String,
    message_count: usize,
}
```

**File Structure**:
```
~/.pawan/sessions/
  abcdef12.json
  ghijkl34.json
  ...
```

---

## Implementation Phases

### Phase 1: Session Persistence Foundation (High Priority)
**Goal**: Deep TUI integration of existing session save/load functionality

#### Tasks

1. **Add Session State to TUI App** ✅
   - Location: `/opt/pawan/crates/pawan-cli/src/tui/mod.rs`
   - Changes:
     - Add `session_id: Option<String>` to `App` struct
     - Add `auto_save: bool` configuration option
     - Add `config.session_persistence` field (default: true)
     - Add `config.auto_save_interval` field (default: 300) - seconds
   
2. **Auto-save on Session Completion** ✅
   - Hook into `AgentEvent::Complete` handler
   - Save session with:
     - Current model name
     - Conversation history (messages)
     - Token usage metrics
     - Iteration count
   - Extract session ID from agent if available, create new one if None
   
3. **Add Session TUI Components**
   - New widget: `render_session_info()` in status bar
   - Status bar shows: `📝 Session: <id> | Tokens: <n> | Files: <n>`
   - Color coding: Gray when no session (~), Green when active session (📝), Red if save failed

4. **Update Slash Commands** ✅
   - `/save` — Force manual save (shows path in status)
   - `/sessions` — Open session browser (like command palette)
   - `/load <id>` — Load and resume session
   - `/clear` — Keep session but clear messages
   - Update help text in `/help` command

5. **Session Browser TUI** ✅
   - Full-screen overlay (Ctrl+S or /sessions)
   - List format: `[✓] abcdef12  qwen-plus  Updated: 2h ago  (15 msg  12345 tokens)`
   - Navigation: Arrow keys, PageUp/Down, Enter to select
   - Filter/search: Type to narrow list (live filter)
   - Actions preview: Show selected session summary on side
   - State management:
     - `session_browser_open: bool`
     - `session_browser_query: String`
     - `session_browser_selected: usize`

6. **First-run Experience** ✅
   - Detect no sessions directory
   - Show welcome banner: "No sessions yet. Any task will create one."
   - Auto-create directory if missing
   
7. **Tests** ✅
   - Unit tests in `mod.rs`:
     - `test_auto_save_on_complete()`
     - `test_session_browser_filter()`
     - `test_session_status_display()`
   - Integration tests via `pawan run --save`

#### Files Modified
```
/opt/pawan/crates/pawan-cli/src/tui/mod.rs
- Add 5 struct fields (~50 LOC added)
- Add 1 enum variant (~5 LOC)
- Add 7 method implementations (~200 LOC)
- Modify 3 existing methods (~50 LOC changed)
```

#### Acceptance Criteria
- [ ] `/save` saves current conversation to new or resumed session
- [ ] Sessions list persists between restarts
- [ ] Status bar shows active session info
- [ ] Auto-save respects config toggle
- [ ] Error handling: disk full, permission denied
- [ ] Unit tests pass in CI
- [ ] Integration tests verify save/load roundtrip

---

### Phase 2: Model Selector Enhancement (High Priority)
**Goal**: Replace manual `/model <name>` typing with interactive model browser

#### Current State 📊
- Model switching already supported
- `/model qwen-plus` works but not user-friendly
- Config supports `model` field in pawan.toml

#### Tasks

1. **Enhance App State** ✅
   - Location: `/opt/pawan/crates/pawan-cli/src/main.rs` and `tui/mod.rs`
   - Changes:
     - Add `models: Vec<String>` to config (injected from `PawanConfig::available_models()`)
     - Cache models list statically to avoid repeated discovery
     - Add `model_selector_open: bool` to `App` struct
     - Add `model_selector_query: String`
     - Add `model_selector_selected: usize`
   
2. **Model Discovery Service** ✅
   - Create trait: `pawan_core::config::ModelResolver`
   - Implementations:
     - `DirmacsModelResolver` — uses dirmacs sklearn API endpoint
     - `LocalModelResolver` — reads from `~/.pawan/models.toml`
     - `OpenAIModelResolver` — queries OpenAI API models list
     - `OllamaModelResolver` — reads local Ollama instanced
   - Response format: `Vec<(name, provider, max_tokens, description)>`

3. **Model Selector TUI** ✅
   - Trigger: `/model` (shows browser) or `/m`
   - Hotkey binding (once): Shift+M or Ctrl+M for instant access
   - Multi-column list:
     
     ```
     ┌── Model Selector ─────────┐
     │ 🔍 Qwen  → Looks like... │
     │ • gpt-4o              │
     │ • gpt-4o-mini           │
     │ • o1-preview            │
     │ • qwen-plus              │
     │ • qwen-max               │
     │ • deepseek-v3            │
     │ • ...                   │
     │                          │
     │ Status: 12 models found  │
     │[Enter] Select  [Esc] Exit│
     └──────────────────────────────┘
     ```
   - Columns: Provider icon, Model name, Quick stats, Description preview
   - Filter: Live search as you type
   - Sort: Alphabetical, then by recency (default: Default model first)

4. **Selection Flow** ✅
   ```
   User types: /model
   → Opens model selector overlay
   → User navigates with arrows/JK/LM
   → Presses Enter on selection
   → Sends AgentCommand::SwitchModel(selected)
   → Receives updated model name in status bar
   → Session uses new model for future interactions
   ```

5. **Display Integration** ✅
   - Status bar shows: `🤖 Model: <name>` (blue)
   - Hover info: Shows model details on mouse-over (if mouse enabled)
   - `/model` command logs: "Switched model to: <name>"

6. **Favorites & Recent** ✅
   - Default model pre-selected in list
   - Recent models cached in config's `recent_models` array
   - Quick access: Top 3 models shown first
   - Truncate long descriptions to 40 chars in list view

#### Files Modified
```
/opt/pawan/crates/pawan-cli/src/main.rs
  - Add ModelResolver trait and implementations (estimate: 200 LOC new)

/opt/pawan/crates/pawan-cli/src/tui/mod.rs
  - Add model_selector_* fields (~50 LOC)
  - Add model_selector_render() method (~150 LOC)
  - Modify handle_event() to handle selector activation (~20 LOC changed)
  - Update /model command handler (refine existing, ~10 LOC)
```

#### Acceptance Criteria
- [ ] Model selector opens via `/model` and Shift+M
- [ ] Shows available models with metadata
- [ ] Filtering works live (no Enter required)
- [ ] Selection updates model in 1s
- [ ] Recent models prioritized
- [ ] Default model highlighted
- [ ] Tests verify UI navigation
- [ ] Config persists recent model list

---

### Phase 3: Session Management TUI Integration (Medium Priority)
**Goal**: Make sessions first-class citizens in the TUI workflow

#### Tasks

1. **Session Browser Overlay** ✅
   - Full-screen modal with:
     - List of sessions (like command palette)
     - Live filter/search
     - Session summary pane on right:
       - Model used
       - Token count breakdown
       - Date/time stamps
       - Message preview (first 2 visible lines)
     - Header with total sessions count
     - Footer with command hints
   - Navigation bindings identical to message scrolling (j/k, arrow keys)
   - Selection confirmation: Enter loads session, Esc cancels

2. **Session Manager Commands** ✅
   
   | Command | Short | Behavior |
   |---------|-------|----------|
   | `/sessions` | `/ses` | Open session browser |
   | `/save` | `/s` | Force save current session |
   | `/load <id>` | `/l` | Resume session by ID |
   | `/delete <id>` | `/d` | Delete session (confirmation dialog) |
   | `/export <id> <path>` | `/e` | Export session to markdown |
   | `/prune [older-than-days]` | | Auto-delete sessions older than 30d (defaults) |

3. **Auto-save Enhancements** ✅
   - Auto-save on: 
     - Agent completion (existing)
     - Tool completion that edits files
     - Session switch (model change)
   - Auto-save interval: `config.auto_save_interval` (default 300s)
   - Session lifetime: Uses last-used session if available, else creates new
   - State tracking: `session_id` stored in App, loaded from agent

4. **Session Status Bar Widget** ✅
   ```
   📝 Session: abcdef12 | 🤖 Model: qwen-plus | Tokens: 12345 | Files: 7
   ```
   - Components:
     - 📝 icon = session active
     - Session ID (clickable to open browser)
     - Model name (hover for provider)
     - Token metrics (total tokens)
     - File edit count (work done visualization)
   - Colors: Session ID (cyan), Model (yellow), Tokens (green), Files (magenta)

5. **Session Info Dialog** ✅
   - Press `i` while messages panel focused to show session details
   - Modal popup with:
     - Summary statistics (all metrics)
     - Tool usage breakdown
     - All tokens used
     - Created/restarted timestamps
     - Quick stats bar chart ASCII
   - Close with Esc or Enter

6. **Export Integration** ✅
   - `/export <path>` saves full Markdown with:
     - Session header with metadata
     - Conversation threads in order
     - Tool call logs with success/failure icons
     - Token usage summary
     - Model provenance
   - Default path: `pawan-<id>-<timestamp>.md` in `~/.pawan/exports/`

#### Files Modified
```
/opt/pawan/crates/pawan-cli/src/tui/mod.rs
  - Add session_browser_* fields (~50 LOC)
  - Add session_browser_render() method (~200 LOC)
  - Add render_session_info() method (~100 LOC)
  - Add render_session_detail() helper (~80 LOC)
  - Modify ui() to include status bar widget (~10 LOC)
  - Extend slash command handlers (~40 LOC)
```

#### Acceptance Criteria
- [ ] `/sessions` shows interactive browser
- [ ] Sessions sorted by updated_at (newest first)
- [ ] Filter works live, matches model names and IDs
- [ ] Selecting loads session via API
- [ ] Status bar updates in <1s
- [ ] `i` key shows detailed session info
- [ ] `/export` saves to specified path
- [ ] All sessions accessible within 3-dir depth

---

### Phase 4: Polish & Oh-My-Pi Inspiration (Low Priority)
**Goal**: Elevate UX with omp-inspired session metadata display and workflow polish

#### Tasks

1. **Session Metadata Display** 🎯
   - Study omp's session display in status bar
   - Add:
     - Elapsed time since session started
     - Session tagging/categorization (manual: `#bug`, `#feature`, `#refactor`)
     - Task completion percentage (files edited / files in project)
     - Self-rating widget (user can rate session 1-5 ⭐)
   - Save metadata to session JSON

2. **Session Categories/Tags** 🎯
   - Manual tag insertion: `Set tag: #performance #bug #needs-docs`
   - Auto-detection: Tags inferred from:
     - Tool call names (`write_file` → `#refactor`, `edit_file` → `#improvement`)
     - File extensions edited
   - Filter by tag in session browser: `/tag performance`

3. **Session Search & Export** 🎯
   - Add `/search` with `--session-tag` filter
   - Export options:
     - Markdown (existing)
     - JSON with structured data
     - ZIP bundle (md + screenshots if available)
     - HTML with animated tool call timeline

4. **Auto-prune Sessions** 🎯
   - Config: `config.session_ttl_days` (default: 30)
   - Background maintenance: cleanup runs on pawan startup
   - Notification: "Auto-pruned X old sessions"
   - Preserve: Only keep sessions modified in last 30 days

5. **Enhanced Status Bar** 🎯
   - Multi-row status bar:
     ```
     Row 1: Session + Model + Token Metrics
     Row 2: Active tool execution status + elapsed time
     Row 3: Session tag badges + quick filter info
     ```
   - Icons library: Using nerd font emoji subsystem if detected
   - Dynamic width allocation: Uses max available width

6. **First-time Tutorial** 🎯
   - Detect fresh install (no sessions)
   - Show welcome tour:
     - "Try: Describe a coding task and hit Enter"
     - "Save: Use /save to checkpoint conversation"
     - "Sessions: /sessions to browse past work"
     - "Model: /model to switch LLM"
   - Auto-advance after any key or 10s timeout

#### Files Modified
```
/opt/pawan/crates/pawan-cli/src/tui/mod.rs (additive)
  - Add tags: Vec<String> to Session and App (~30 LOC)
  - Refine ui() layout to 3-row status bar (~20 LOC)
  - Add tutorial overlay rendering (~120 LOC)
  - Enhance export_conversation() with options (~50 LOC)
```

---

## Technical Architecture

### Components Diagram

```
┌────────────────────────┐   ┌────────────────────────┐
│   pawan CLI Binary   │   │   pawan-core Lib    │
│   (crate:pawan-cli) │   │   (crate:pawan-core)│
└────────┬──────┬───────┘   └────────┬──────┬──────┘
         │      │                    │      │
  ┌──────┴─┐ ┌──┴──────┐    ┌───┴─┐ ┌──┴──────┐
  │ TUI Mod │ │Main.rs  │    │Agent│ │Session  │
  │(mod.rs) │ │         │    │Logic│ │Storage │
  └────┬────┘ └─────────┘    └─────┘ └─────────┘
       │                            │
  ┌────┴────┐                  ┌───┴────┐
  │   Rust  │                  │ JSON    │
  │tui-rs   │                  │ Files   │
  └─────────┘                  └─────────┘
```

### Data Flow

**Session Save**:
```
User Input → App::submit_input()
  → AgentCommand::Execute() → Agent
  → On Complete → App handles AgentEvent::Complete
    → Extract all messages from agent
    → Create Session::new(agent.model)
    → Save Session to ~/.pawan/sessions/{id}.json
    → Update status bar widget
```

**Model Selector**:
```
Type "/model" → App enters model selector mode
  → Render ModelBrowser widget
  → Read models from config.model_resolver_models()
  → Format as list
  → User navigation → App tracks selection
  → On Enter → App sends AgentCommand::SwitchModel(selected)
  → Agent updates internal model and responds
```

**Session Load**:
```
Type "/load abcdef12" → App parses command
  → PawanAgent::resume_session(id) [pawan-core]
  → Agent loads Session from file
  → Agent restores messages and state
  → App receives updated message list
  → App renders messages in UI
```

### Configuration Additions

```toml
# ~/.pawan/pawan.toml - new additions

[ui]
session_persistence = true
auto_save_interval = 300  # seconds

[models]
default_model = "qwen-plus"
recent_models = [
  "qwen-plus",
  "deepseek-v3",
  "gpt-4o"
]

[sessions]
ttl_days = 30
auto_prune_on_startup = true

[ui.status_bar]
compact = false  # 3-row vs single row
```

---

## Risk & Mitigation

### Risks 📊

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Session corruption on disk full | High | Check disk space before save, graceful error in status bar |
| Model fetch API timeout | Medium | Cache models list client-side, fallback to local list |
| Session ID collision (8-char) | Low | UUID v4 prefix → 32-bit collision domain = 65k sessions before 50% chance, daily usage unlikely to hit |
| TUI lag on 100+ sessions | High | Pagination (20/screen) with lazy loading, filter to reduce list size |
| Race condition on concurrent saves | Medium | Session lock per-ID, atomic write via temp file → rename |
| Migration on format change | Low | Version in session JSON, migration path in Session::load() |

### Mitigation Details

**Disk Space Check**:
```rust
fn check_disk_space(path: &Path) -> bool {
    let available = std::fs::metadata(path)
        .map(|m| m.available_space())
        .unwrap_or(1_000_000); // 1MB guardrail
    available > 10_000_000 // 10MB minimum
}
```

**Session Lock**:
```rust
let _lock = FileLock::new(&session_path).lock();
let temp_path = session_path.with_extension("tmp");
std::fs::write(&temp_path, json)?;
std::fs::rename(&temp_path, &session_path)?; // atomic
```

---

## Performance Considerations

### Time Complexity

- **Session List**: O(n log n) for sorting, n = session count ✅
- **Session filter**: O(n) live filtering, updates per keystroke ✅
- **Model list fetch**: O(m) once, cached globally ✅
- **Session save**: O(1) file write, small JSON (<100KB) ✅

### Memory Usage

- App state: ~500 bytes per session reference (in list)
- 1000 sessions = ~500KB, negligible ✅
- Model list cached: ~1KB per model × 50 models = 50KB ✅
- Active session messages: Deduplicated via agent state, not duplicated in UI ✅

---

## Testing Strategy

### Unit Tests 🧪

**tui/mod.rs**:
```rust
#[test]
fn test_session_save_command_saves_to_disk() { /* ... */ }

#[test]
fn test_model_selector_highlights_recent_models() { /* ... */ }

#[test]
fn test_filter_sessions_by_query() { /* ... */ }
```

**pawan-cli/main.rs**:
```rust
#[test]
fn test_config_models_loads_correctly() { /* ... */ }

#[test]
fn test_auto_save_respects_config() { /* ... */ }
```

### Integration Tests 🧪

**Manual workflow**:
```bash
# Create session
pawan task "fix compilation error in main.rs" --save

# Verify
pawan sessions

# Resume
pawan chat --resume $(jq -r '.id' ~/.pawan/sessions/*.json)

# Delete
pawan sessions
```

**Test scenarios**:
1. Create 3 sessions, verify list shows all sorted
2. Switch model 5 times, verify recent_models updated
3. Auto-save after 10 messages, verify file exists
4. Filter session list to "feature", verify only matching shown
5. Select session, verify message display updates

### Regression Tests 🧪

- Existing `pawan test` suite continues to pass
- CI runs `cargo test` in pawan-core and pawan-cli
- Verify no performance regression in TUI rendering

---

## Success Metrics

### User Experience 📈
- **Session completion rate**: % of conversations that use `/save` or get auto-saved
- **Session return rate**: % of users who return to previous session within 7 days
- **Model switch frequency**: # /hour/session for selector usage
- **Time-to-first-task**: Seconds from launch to productive coding

### Technical 📈
- CI test coverage: >90% on new code paths
- No regressions in existing test suite
- TUI frame rate: >30fps on typical VPS terminal
- Memory growth: <5% per session launched
- Disk usage: <5MB per 100 sessions

---

## Implementation Order & Priority Matrix

| Feature | Priority | Phase | Effort (days) | Dependencies |
|---------|----------|-------|----------------|--------------|
| Auto-save integration | 🔴 P0 | 1 | 2 | Session infra exists |
| Session browser TUI | 🔴 P0 | 1 | 3 | App state hooks |
| Model selector TUI | 🟡 P1 | 2 | 2 | Config API |
| `/save`, `/load` commands | 🟡 P1 | 1 | 1 | Session infra |
| Status bar widget | 🟡 P1 | 3 | 1 | UI refactor |
| Tags/categories | 🟢 P2 | 4 | 2 | Session schema |
| Search/filter | 🟢 P2 | 4 | 1 | UI concurrency |
| Export options (JSON) | 🟢 P2 | 4 | 1 | User feature |
| Auto-prune sessions | 🟢 P2 | 4 | 1 | Session bag |

---

## Rollback Plan

### If issues found:
1. Session saves failing:
   - Merge with safeguard: set `session_persistence = false` in config
   - Auto-disable auto-save on disk error

2. Model selector hangs:
   - Fallback to `/model <name>` simple syntax
   - Log error to status bar: "Model fetch failed, using cached list"

3. TUI performance degradation:
   - Add pagination: 50 sessions/page
   - Implement lazy loading for session summaries
   - Add `compact = true` config to reduce status bar rows

4. Data corruption detected:
   - Session load guards: validate JSON schema v3
   - Fallback path: preserve old sessions in `corrupted/` namespace

---

## Migration & Compatibility

### Existing Users
- No breaking changes
- Existing sessions: Migration via `Session::load()` handles missing fields (backwards compatible)
- New features: Opt-in via config flags
- Old config: pawan.toml unchanged except for new additive fields

### New Users
- First run: Sessions dir auto-created
- Configuration: pawan.toml generated with new defaults
- Tutorial: Welcome screen shown once

---

## Resources & References

### pawan-core Session API
```rust
Session::new(model: &str) -> Session
Session::save(&mut self) -> Result<PathBuf>
Session::load(id: &str) -> Result<Session>
Session::list() -> Result<Vec<SessionSummary>>
Session::sessions_dir() -> Result<PathBuf>
```

### Agent Commands (Existing)
```rust
enum AgentCommand {
    Execute(String),
    SwitchModel(String),  // ← Already available
    Quit,
}
```

### TUI Events (Existing)
```rust
enum AgentEvent {
    Token(String),
    ToolStart(String),
    ToolComplete(ToolCallRecord),
    Complete(Result<AgentResponse, PawanError>),
    // ...
}
```

---

## Sign-off Checklist 📋

### Pre-review
- [ ] Code follows Rust conventions (rustfmt)
- [ ] No unsafe blocks introduced
- [ ] Error handling uses PawanError type
- [ ] Logging via tracing crate where appropriate
- [ ] No panics in hot paths

### Testing
- [ ] Unit tests in mod.rs cover new code
- [ ] Integration tests verify save/load roundtrip
- [ ] Manual testing on VPS terminal
- [ ] CI passes: `cargo test --all`

### Documentation
- [ ] Inline code comments for non-obvious logic
- [ ] Command help text updated
- [ ] Status bar text culturally consistent (no offensive terms)
- [ ] Feature parity with spec documented

### Polish
- [ ] TUI rendering is clean (no artifacts or corruption)
- [ ] Keybindings are mnemonic and discoverable
- [ ] Filtering feels instantaneous (<200ms on 100 sessions)
- [ ] Status bar updates within 500ms of event

### Handoff
- [ ] Commit messages reference this plan
- [ ] Git author set to `bkataru <baalateja.k@gmail.com>`
- [ ] PR includes screenshots of new features
- [ ] Changelog entry written (if applicable)

---

## Appendix A: Example Session Browser Screens

### Empty State
```
╔═══════════════════════════╗
║   Sessions Browser  (0 total)     ║
╚═══════════════════════════╝

💡 No sessions yet. Any coding task will create one.

[Create new session]  ___  

Type to search…

Press [Esc] to exit
```

### With Sessions
```
╔═══════════════════════════╗
║   Sessions Browser  (5 total)     ║
╚═══════════════════════════╝

🔍 feature im

✓ abcdef12 | qwen-plus | 2h ago | 15 msg | 12,345 tokens
✓ ghijkl34 | deepseek-v3 | 1d ago | 8 msg | 8,901 tokens
• mnopqr56 | gpt-4o | 3d ago | 23 msg | 18,002 tokens
• stuvwx78 | qwen-max | 1w ago | 4 msg | 4,890 tokens
───────────────────────────────────────

┌─ Session Summary ───────────┐
│ Model: qwen-plus          │
│ Total tokens: 12,345     │
│ Prompt: 8,221           │
│ Completion: 4,124        │
│ Reasoning: 1,012         │
│ Action: 3,112            │
│ Files edited: 7           │
│ Tags: #performance #bug   │
│ Created: 2026-04-13 10:00│
│ Updated: 2026-04-13 12:34│
└──────────────────────────────┘

↑↓ Navigate | Enter Load | / Filter | Esc Exit
```

---

## Appendix B: Model Selector Screens

### Instant Access (Shift+M)
```
╒═══════════════ Model Selector ═══════════════╕
│ 🔍 gpt →                                                 │
│ • gpt-4o  (OpenAI, 128k ctx, fast)                 │
│ ✓ qwen-plus (Qwen, 32k ctx, reasoning heavy)          │
│ • deepseek-v3 (DeepSeek, 64k ctx, logic)              │
│ • qwen-max   (Qwen, 128k ctx, general)              │
│ • o1-preview (OpenAI, chain-of-thought)                │
│ • gemini-1.5-pro (Google, multimodal)                   │
╘═════════════ 6 models found ══════════════╛

Enter: Select  Esc: Exit  Type: Live filter
```

### Detailed View (After Selection)
```
┌─ Model Details ────────────────┐
│ Name: qwen-plus               │
│ Provider: Qwen                │
│ Context: 32,000 tokens        │
│ Max output: 16,000 tokens     │
│ Speed: ~20 tok/s              │
│ Cost: $0.001 / 1K tokens     │
│ Features: Reasoning, Tool use  │
│ Status: Ready ✓                │
└──────────────────────────────────┘
```

---

## Appendix C: Configuration Snippets

### Full pawan.toml Example
```toml
[ui]
session_persistence = true
auto_save_interval = 300

[models]
default_model = "qwen-plus"
recent_models = [
  "qwen-plus",
  "deepseek-v3",
  "gpt-4o",
  "qwen-max"
]

[sessions]
ttl_days = 30
auto_prune_on_startup = true
prune_older_than_days = 60

[ui.status_bar]
rows = 3  # 1, 2, or 3
compact = false
color_scheme = "auto"
```

### Minimal (explicit defaults)
```toml
# ~/.pawan/pawan.toml
[models]
default_model = "qwen-plus"
```

---

## Appendix D: Command Reference

| Command | Syntax | Description |
|---------|--------|-------------|
| Save | `/save`, `/s` | Force save current session |
| Sessions | `/sessions`, `/ses` | Open session browser |
| Load | `/load <id>`, `/l <id>` | Resume session by ID |
| Export | `/export [path]`, `/e [path]` | Save conversation to Markdown |
| Delete | `/delete <id>`, `/d <id>` | Delete session (asks confirmation) |
| Clear | `/clear`, `/c` | Clear messages but keep session |
| Model | `/model [name]`, `/m [name]` | Show or switch model |
| Tools | `/tools`, `/t` | List available tools |
| Heal | `/heal`, `/h` | Auto-fix compilation issues |
| Quit | `/quit`, `/q` | Exit pawan |

---

## Appendix E: Disk Layout

```
~/.pawan/
├── sessions/
│   ├── abcdef12.json
│   ├── ghijkl34.json
│   └── mnopqr56.json
├── exports/
│   ├── pawan-abcdef12-2026-04-13T12-34.md
│   └── ...
├── pawan.toml
└── logs/
    ├── 2026-04-13_session_startup.log
    └── ...
```

### Session JSON Schema
```json
{
  "id": "abcdef12",
  "model": "qwen-plus",
  "created_at": "2026-04-13T10:00:00Z",
  "updated_at": "2026-04-13T12:34:56Z",
  "messages": [ /* Array of Message objects */ ],
  "total_tokens": 12345,
  "iteration_count": 7,
  "tags": ["#performance", "#bug"]
}
```

---

## Final Notes

### From Requirements{}