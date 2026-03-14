+++
title = "Commands"
+++

## Interactive

### `pawan`

Default TUI mode. Streaming markdown rendering, vim keybindings, token tracking.

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

## Configuration

### `pawan config show`

Display the fully resolved configuration.

### `pawan config init`

Generate a `pawan.toml` template.

### `pawan mcp list`

List connected MCP servers and their discovered tools.

### `pawan completions <shell>`

Generate shell completions for bash, zsh, fish, etc.
