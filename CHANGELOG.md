## [0.5.0] - 2026-04-27

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
