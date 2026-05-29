¶/opt/pawan/CHANGELOG.md#90BA
1:## [0.5.7] - 2026-05-29

### Refactoring
- Split `app.rs` into `app/{state,constructors,model_ops,session_ops,async_ops,mod}.rs`
- Split `agent/mod.rs` into `{construction,execute,session,session_store}.rs`
- Split `config/mod.rs` (~1792 LOC) into 13 smaller modules
- Refactored `App::handle_event` into `events/` submodule (8 files)
- Completed `slash_handlers.rs` (853 lines, 18 handlers) and `slash_commands.rs` (132 lines)

### Features
- Dynamic model catalog with async NVIDIA API fetch, 5s timeout, hardcoded fallback
- `/goal`, `/loop`, `/orchestrate` slash commands for advanced agent modes
- IRC chat and subagent coordination (`IrcHub`, `IrcRelay`, `IrcMessage`)
- Hindi welcome popup replacing maki splash screen; cyan-accent default theme

### Tests
- 1643+ tests passing across 18 suites (up from 988+)
- Added SubagentCoreTests, ModelCatalogTests, IrcHubTests, CommitQueuePanel, CoordinatorTests, GoalLoopRuntime test suites

### Fixes
- Removed all `-D warnings` violations (unused imports, dead code)
- URL validation added to `LancorBackend` constructors

## [0.5.6] - 2026-04-29
2:
3:### Tests
4:- Added 98 new tests: 29 compaction tests (edge cases, strategies, keywords, summary parsing), 16 eruka bridge tests (serde, JSON parsing, disabled client paths), and 53 TUI types tests (format parsing, strip_reasoning_tags, ContentBlock, ToolBlockState)
5:- Total: 988+ tests passing, 74.58% line / 74.80% region / 77.34% function coverage (cargo-llvm-cov baseline)
6:- Fixed `strip_reasoning_tags` test coverage to use the correct `<think>` tag format (was incorrectly using `<reasoning>`)
7:- Added `lib.rs` section to pawan-cli for integration test access; exposed `ContentBlock`, `ToolBlockState`, and helper functions as `pub` (not `pub(crate)`)
8:- Fixed eruka string literal encoding in test file: binary `"` characters now written correctly via Python script (bash heredoc HTML entities were producing literal `&quot;` bytes)
9:
10:## [0.5.5] - 2026-04-28
11:
12:### Fixed
13:- `/theme <name>` now submits correctly from the TUI input when pressing Enter; the inline slash picker no longer intercepts commands once arguments are present.
14:- Input placeholder text now uses the active theme's readable muted color on startup, after resets, and after theme switches.
15:- Bottom status bar polish: model, token count, context percentage/bar, iteration, and timestamp now have visible separators and spacing.
16:
17:### Tests
18:- Added TUI regression coverage for typed slash-command submission, `/theme` variants, theme help/error paths, textarea placeholder styling, and status bar spacing.
19:
20:## [0.5.4] - 2026-04-28
21:
22:### Fixed
23:- TUI visual containment restored: main interface now renders inside a framed shell with an outer gutter instead of running edge-to-edge.
24:- Dark-mode readability improved: secondary text, timestamps, tool metadata, status bar details, and scroll indicators now use readable theme tokens instead of low-contrast dark gray.
25:- Inline slash command picker fixed: selecting commands such as `/m` and `/theme` with Enter now dispatches the selected command directly.
26:- `/theme` with no arguments now prints available themes and usage in the transcript.
27:
28:## [0.5.3] - 2026-04-28
29:
30:### Changed
31:- TUI redesign: activity panel removed, full-width chat with inline tool activity
32:- Status bar moved to bottom with mode badge, thinking label, git branch, model name, token bar, iteration, timestamp
33:- Borderless input and message areas with subtle scroll % and search hint overlays
34:- Dead code removed: `activity_panel.rs`, `show_activity_panel` field, `render_activity()`, `render_messages_with_activity()`
35:- Duplicate SVGs removed from `docs/img/` (identical copies of `docs/static/`)
36:
37:### Fixed
38:- Stale version references updated across all README and docs files (v0.5.0 → v0.5.3)
39:- `pawan-web` health response version updated from 0.4.8 to 0.5.3
40:- `pawan-aegis` default model updated to `qwen/qwen3.5-122b-a10b`
41:
42:## [0.5.2] - 2026-04-28
43:
44:### Fixed
45:- gix: upgraded from 0.82 to 0.83 to resolve yanked dependency (gix-actor also updated to 0.41)
46:
47:## [0.5.1] - 2026-04-27
48:
49:### Fixed
50:- ColorTransition: `/theme` now animates accent color (focus borders, input title bar) via `set()` instead of instant-snap `new()`
51:- TUI focus borders: hardcoded `Color::Cyan` replaced with `accent_transition.resolve()` for animated theme transitions
52:- render_status: replaced with `StatusBar::view()` across both layout paths; dead `keybind_status_hint` and `KeyAction` removed
53:
54:### Added
55:- StatusBar component: rich status strip with flash-on-event, mode badge (INPUT/NORMAL/CMD/HELP/MODEL), context bar, iteration counter, timestamp
56:- `status_bar.flash()` integrated into `/theme` slash command on successful theme switch
57:- `⚡` animation indicator in input area title bar while accent color transition is in progress
58:- `KeybindContext` enum variants exposed in mode badge: Input, Normal, Command, Help, ModelPicker
59:
60:### Added
61:- Doom-loop detection with configurable backoff multiplier and automatic reset
62:- Retry policy with exponential backoff and jitter
63:- Cancellation history hygiene with `sanitize_cancelled_history`
64:- Auto-compaction with LLM summarization
65:- Parallel tool execution with bounded concurrency and `max_parallel_tools`
66:- Batch tool supporting up to 25 concurrent calls
67:- Bash permission tiers (feature-gated, tree-sitter based)
68:- Tool audience bitflags (MAIN, SUB, LUA)
69:- Subagent task tool (six agent types, depth 1, 300s timeout)
70:- Agent definitions with YAML frontmatter and markdown
71:- Concurrent agent pool with semaphore bounding
72:- SQLite session store in WAL mode with FTS5 and JSON migration
73:- JSONL session branching with `parent_id` and branch depth capped at 5
74:- Session labels and bookmarks
75:- Reasoning tag stripping with `strip_reasoning_tags` (regex dependency)
76:- Keybind contexts via `KeybindContext` and mode transitions
77:- Model picker modal (Ctrl-M, provider badges, scrollable list)
78:- Fuzzy search modal (Ctrl-P, substring filter, scrollable list)
79:- `--print` headless mode: print the final response and skip the TUI
80:- `--output-format` with `text`, `json`, and `stream-json`
81:- Slash command registry: `/model`, `/session`, `/clear`, `/retry`, `/compact`, `/help`
82:- `--continue` to resume the most recent session
83:- `--session <id>` to continue a specific session
84:- `--list-sessions` / `-l` listing sessions in a table with metadata
85:- Heuristic memory extraction from conversation with repetition detection
86:- Memory consolidation (merge by key, prune old low-relevance entries)
87:- Memory retrieval via Jaccard similarity with context injection
88:- Prompt injection scanner with six detection patterns
89:- Memory fencing with `SessionScopedMemory` and sanitize/validate for keys and content
90:
91:### Changed
92:- TUI split into seven submodules; `mod.rs` reduced to a small facade (~20 lines)
93:- CLI extended with headless, session selection, and structured output options alongside interactive TUI
94:
95:### Fixed
96:- Hardening around session branching limits, memory sanitation, and cancellation/retry interaction paths
97: