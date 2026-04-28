## [0.5.4] - 2026-04-28

### Fixed
- TUI visual containment restored: main interface now renders inside a framed shell with an outer gutter instead of running edge-to-edge.
- Dark-mode readability improved: secondary text, timestamps, tool metadata, status bar details, and scroll indicators now use readable theme tokens instead of low-contrast dark gray.
- Inline slash command picker fixed: selecting commands such as `/m` and `/theme` with Enter now dispatches the selected command directly.
- `/theme` with no arguments now prints available themes and usage in the transcript.

## [0.5.3] - 2026-04-28

### Changed
- TUI redesign: activity panel removed, full-width chat with inline tool activity
- Status bar moved to bottom with mode badge, thinking label, git branch, model name, token bar, iteration, timestamp
- Borderless input and message areas with subtle scroll % and search hint overlays
- Dead code removed: `activity_panel.rs`, `show_activity_panel` field, `render_activity()`, `render_messages_with_activity()`
- Duplicate SVGs removed from `docs/img/` (identical copies of `docs/static/`)

### Fixed
- Stale version references updated across all README and docs files (v0.5.0 → v0.5.3)
- `pawan-web` health response version updated from 0.4.8 to 0.5.3
- `pawan-aegis` default model updated to `qwen/qwen3.5-122b-a10b`

## [0.5.2] - 2026-04-28

### Fixed
- gix: upgraded from 0.82 to 0.83 to resolve yanked dependency (gix-actor also updated to 0.41)

## [0.5.1] - 2026-04-27

### Fixed
- ColorTransition: `/theme` now animates accent color (focus borders, input title bar) via `set()` instead of instant-snap `new()`
- TUI focus borders: hardcoded `Color::Cyan` replaced with `accent_transition.resolve()` for animated theme transitions
- render_status: replaced with `StatusBar::view()` across both layout paths; dead `keybind_status_hint` and `KeyAction` removed

### Added
- StatusBar component: rich status strip with flash-on-event, mode badge (INPUT/NORMAL/CMD/HELP/MODEL), context bar, iteration counter, timestamp
- `status_bar.flash()` integrated into `/theme` slash command on successful theme switch
- `⚡` animation indicator in input area title bar while accent color transition is in progress
- `KeybindContext` enum variants exposed in mode badge: Input, Normal, Command, Help, ModelPicker

### Added
- Doom-loop detection with configurable backoff multiplier and automatic reset
- Retry policy with exponential backoff and jitter
- Cancellation history hygiene with `sanitize_cancelled_history`
- Auto-compaction with LLM summarization
- Parallel tool execution with bounded concurrency and `max_parallel_tools`
- Batch tool supporting up to 25 concurrent calls
- Bash permission tiers (feature-gated, tree-sitter based)
- Tool audience bitflags (MAIN, SUB, LUA)
- Subagent task tool (six agent types, depth 1, 300s timeout)
- Agent definitions with YAML frontmatter and markdown
- Concurrent agent pool with semaphore bounding
- SQLite session store in WAL mode with FTS5 and JSON migration
- JSONL session branching with `parent_id` and branch depth capped at 5
- Session labels and bookmarks
- Reasoning tag stripping with `strip_reasoning_tags` (regex dependency)
- Keybind contexts via `KeybindContext` and mode transitions
- Model picker modal (Ctrl-M, provider badges, scrollable list)
- Fuzzy search modal (Ctrl-P, substring filter, scrollable list)
- `--print` headless mode: print the final response and skip the TUI
- `--output-format` with `text`, `json`, and `stream-json`
- Slash command registry: `/model`, `/session`, `/clear`, `/retry`, `/compact`, `/help`
- `--continue` to resume the most recent session
- `--session <id>` to continue a specific session
- `--list-sessions` / `-l` listing sessions in a table with metadata
- Heuristic memory extraction from conversation with repetition detection
- Memory consolidation (merge by key, prune old low-relevance entries)
- Memory retrieval via Jaccard similarity with context injection
- Prompt injection scanner with six detection patterns
- Memory fencing with `SessionScopedMemory` and sanitize/validate for keys and content

### Changed
- TUI split into seven submodules; `mod.rs` reduced to a small facade (~20 lines)
- CLI extended with headless, session selection, and structured output options alongside interactive TUI

### Fixed
- Hardening around session branching limits, memory sanitation, and cancellation/retry interaction paths
