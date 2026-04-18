+++
title = "Commands"
+++

## Interactive

### `pawan`

Default TUI mode with ratatui-powered interface.

Features: welcome screen, command palette (`Ctrl+P`), F1 help overlay, split layout with activity panel, streaming markdown with interleaved tool call display, inline slash command popup, vim-like navigation (`j/k`, `g/G`, `Ctrl+U/D`, `PageUp/PageDown`, `/search`, `n/N`), mouse wheel scroll support, expand/collapse tool results (`e`), message timestamps, scroll position indicator, session tags (green), fuzzy session search (`[FUZZY]` indicator), session stats, conversation export.

### TUI Slash Commands

| Command | Shorthand | Description |
|---------|-----------|-------------|
| `/model [name]` | `/m` | Show or switch LLM model |
| `/models` | | Browse NVIDIA NIM model catalog |
| `/search <query>` | `/s` | Web search via Daedra MCP |
| `/ss <query>` | | Search saved sessions by content |
| `/prune [args]` | | Prune old sessions (e.g., 30d, 100s) |
| `/tag <cmd>` | | Manage session tags (add/rm/list/clear) |
| `/tools` | `/t` | List available tools by tier |
| `/heal` | `/h` | Auto-fix build errors |
| `/fork` | | Clone current session to a new one |
| `/dump` | | Copy conversation to clipboard |
| `/share` | | Export session and print shareable path |
| `/diff [--cached]` | | Show git diff of working directory (use `--cached` for staged changes) |
| `/handoff` | | Generate focused context for new session |
| `/export [path]` | `/e` | Export conversation to markdown |
| `/import <path>` | | Import session from JSON file |
| `/save` | | Save current session |
| `/load` | | Load a saved session (opens browser if no arg) |
| `/resume` | | Resume a saved session (opens browser if no arg) |
| `/new` | | Start new session |
| `/clear` | | Clear chat history |
| `/quit` | `/q` | Exit pawan |
| `/help` | `/?` | Show help |
### Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Ctrl+P` | Any | Toggle command palette |
| `F1` | Any | Toggle keyboard shortcuts overlay |
| `Ctrl+L` | Any | Clear messages |
| `Ctrl+C` | Any | Quit |
| `Tab` | Any | Switch focus (Input/Messages) |
| `j/k` | Messages | Scroll up/down |
| `g/G` | Messages | Jump to top/bottom |
| `Ctrl+U/D` | Messages | Half-page scroll |
| `PageUp/PageDown` | Messages | Page scroll |
| `/` | Messages | Enter search mode |
| `n/N` | Messages | Next/previous search match |
| `e` | Messages | Expand/collapse nearest tool call result |
| `i` | Messages | Return to input |
| `/` | Input | Open inline slash command popup |
| `Up/Down` | Popups | Navigate items |
| `PageUp/PageDown` | Popups | Bulk scroll |
| `g/G` | Popups | Jump to top/bottom |
| `Enter` | Popups | Select item |
| `Esc` | Popups | Close popup |
| `Mouse wheel` | Any | Scroll (respects active popup) |

### `pawan chat --resume <ID>`

Resume a previously saved session by ID. List sessions with `pawan sessions`.

## Code Actions

### `pawan heal`

Auto-fix compilation errors, clippy warnings, and failing tests.

```bash
pawan heal              # fix everything
pawan heal --errors-only    # only compilation errors
pawan heal --warnings-only  # only clippy warnings
pawan heal --commit         # auto-commit fixes
```

### `pawan task "<description>"`

Execute a coding task with full tool access.

```bash
pawan task "add input validation to CreateUserRequest"
```

### `pawan commit`

AI-powered commit workflow. Aliases: `pawan ai-commit`

```bash
pawan commit          # interactive file selection + message generation
pawan commit -a       # stage all files
pawan commit --dry-run  # preview message only
pawan commit -a -y    # stage all, skip confirmation
```

### `pawan improve <target>`

Improve code quality. Targets: `docs`, `refactor`, `tests`, `all`

```bash
pawan improve docs
pawan improve refactor -f src/config.rs
```

### `pawan test`

Run tests and AI-analyze failures.

```bash
pawan test              # run all tests, report failures
pawan test --fix        # auto-fix failing tests
pawan test -f "config"  # filter by test name
```

### `pawan review`

AI code review with severity levels.

```bash
pawan review            # review all changes
pawan review --staged   # staged changes only
pawan review -f src/lib.rs  # specific file
```

### `pawan explain <query>`

AI explanation of files, functions, or concepts.

```bash
pawan explain src/main.rs
pawan explain "how does the agent loop work"
```

## Automation

### `pawan run`

Headless single-prompt execution for scripting.

```bash
pawan run "fix the compilation errors"
pawan run -f prompt.md --output json --timeout 300
pawan run "..." --save  # save session after completion
```

### `pawan watch`

Continuous monitoring with auto-heal.

```bash
pawan watch                    # check every 10s
pawan watch --interval 30      # check every 30s
pawan watch --commit           # auto-commit fixes
```

## Project

### `pawan init`

Scaffold pawan in a project (creates `PAWAN.md`, `pawan.toml`, `.pawan/`).

### `pawan doctor`

Diagnose setup: API keys, model connectivity, config files, git, tools, MCP servers.

### `pawan status`

Show project health (cargo check, clippy, test results).

### `pawan sessions`

List saved conversation sessions.

### `pawan distill`

Distill a completed session into a reusable SKILL.md file via thulpoff.

```bash
pawan distill                          # distill latest session
pawan distill -s abc123                # distill specific session
pawan distill -o ./skills              # custom output directory
pawan distill --eval                   # distill then evaluate against primary model
pawan distill --refine                 # full distill → eval → refine → eval loop
pawan distill --refine --student-model mistral-small-24b
```

The distilled skill can be loaded back by pawan (or any thulp-compatible agent) to reuse learned patterns.

With `--eval`, pawan runs each test case from the distilled skill through the configured model (or `--student-model`) and reports a pass rate.

With `--refine`, pawan automatically improves the skill content based on failing test cases using thulpoff's RefinementEngine, then re-evaluates and reports the improvement delta.

### `pawan bench`

Run model latency benchmarks via nimakai.

### `pawan notify`

Send notifications via relay service.

```bash
pawan notify "build failed" --channel whatsapp
pawan notify "deploy done" --channel telegram
```

### `pawan fmt`

Format code with cargo fmt and cargo clippy --fix.

```bash
pawan fmt          # format and fix
pawan fmt --check  # check only, no changes
```

### `pawan tasks`

Beads-style task tracking (SQLite at `~/.pawan/beads.db`) with dependencies, memory decay, and ready detection.

```bash
pawan tasks list                       # list all tasks
pawan tasks list --status open --priority 1  # filter by status + priority
pawan tasks create "description"       # create a new bead
pawan tasks update <id> --status in_progress
pawan tasks close <id> --reason "done" # close a bead with a reason
pawan tasks ready                      # show beads whose deps are closed
pawan tasks dep add <bead> <depends_on>    # add dependency
pawan tasks dep rm <bead> <depends_on>     # remove dependency
pawan tasks decay --max-age-days 30    # archive closed beads older than N days
```

## Configuration

### `pawan config show`

Display the fully resolved configuration.

### `pawan config init`

Generate a `pawan.toml` template.

### `pawan mcp list`

List connected MCP servers and their discovered tools.

### `pawan completions <shell>`

Generate shell completions for bash, zsh, fish, etc.
