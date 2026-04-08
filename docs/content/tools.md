+++
title = "Tools"
+++

Pawan ships 29 built-in tools with tiered visibility, auto-install, and dynamic MCP discovery.

## Tool Tiers

Tools are organized into tiers to save LLM prompt tokens:

| Tier | Tools | Visibility |
|------|-------|------------|
| **Core** (7) | bash, read_file, write_file, edit_file, ast_grep, glob_search, grep_search | Always in LLM prompt |
| **Standard** (15) | git tools, agents, list_directory, edit_file_lines, insert_after, append_file | In prompt by default |
| **Extended** (7) | ripgrep, fd, sd, tree, mise, zoxide, lsp | Hidden until first use, then auto-activated |

Extended tools are always executable — they just don't appear in the LLM prompt until the model calls one. This saves ~40% prompt tokens on simple tasks.

**Auto-install**: If a native tool (ast-grep, rg, fd, sd) is missing, pawan auto-installs it via mise on first use. No manual setup needed.

## File Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents (supports line offset/limit) |
| `write_file` | Create or overwrite a file. Auto-creates parent dirs. Path normalization detects double workspace prefix. |
| `edit_file` | String replacement editing (old_string → new_string) |
| `edit_file_lines` | Precise editing with anchor-mode (find line by content, not number) |
| `insert_after` | Block-aware insertion (skips over function/struct bodies) |
| `append_file` | Append content to end of file |
| `list_directory` | List directory contents with file metadata |

## Code Intelligence

| Tool | Description |
|------|-------------|
| `ast_grep` | **AST-level code search and rewrite** — structural patterns via tree-sitter. Multi-language. Fast. |
| `lsp` | **LSP code intelligence** via rust-analyzer — type-aware diagnostics, structural search/replace, symbol extraction. |

### ast-grep — structural patterns (multi-language)

```bash
# Find all unwrap() calls
ast_grep(action="search", pattern="$EXPR.unwrap()", lang="rust", path="src/")

# Replace unwrap() with ? operator in one shot
ast_grep(action="rewrite", pattern="$EXPR.unwrap()", rewrite="$EXPR?", lang="rust", path="src/")

# Find all function signatures
ast_grep(action="search", pattern="fn $F($$$A) { $$$ }", lang="rust", path=".")
```

`$VAR` matches single AST node, `$$$VAR` matches variadic (multiple nodes). Supports rust, python, js, ts, go, c, cpp, java.

### lsp — type-aware intelligence (Rust)

```bash
# Find errors/warnings without cargo check
lsp(action="diagnostics", path=".")

# Structural search with type awareness
lsp(action="search", pattern="$a.foo($b)")

# Type-aware search+replace (knows $a is Option<T>)
lsp(action="ssr", pattern="$a.unwrap() ==>> $a?")

# Parse file symbols with types and hierarchy
lsp(action="symbols", path="src/lib.rs")

# Project-wide analysis stats
lsp(action="analyze", path=".")
```

**When to use which:**
- `ast_grep`: fast, multi-language, no project context needed — use for most edits
- `lsp`: slower, Rust-only, but type-aware — use when you need type system information

## Search Tools

| Tool | Description |
|------|-------------|
| `glob_search` | Find files by glob pattern (e.g., `**/*.rs`) |
| `grep_search` | Search file contents by regex pattern |
| `ripgrep` | Native `rg` wrapper — pattern, type filter, context, case-insensitive, max-depth |
| `fd` | Native `fd` wrapper — find files by name/extension/type with max-depth and max-results |

## Shell & System

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with configurable timeout |
| `sd` | Native `sd` wrapper — find-and-replace in files (fixed strings or regex) |
| `tree` | Filesystem tree with disk usage, line counts, metadata |
| `mise` | **Polyglot tool/runtime/task/env manager** — install tools, run tasks, manage envs |
| `zoxide` | Smart directory navigation — query, add, list paths |

### tree — filesystem intelligence

```bash
# Lines of code per directory
tree(path="src/", disk_usage="line", layout="inverted")

# Find large files with human-readable sizes
tree(path=".", sort="size", human=true)

# Flat file listing (for piping)
tree(path=".", layout="flat", pattern="*.rs")

# Extended metadata: permissions, owner, timestamps
tree(path=".", long=true, hidden=true)
```

### mise — tool/task/env manager

```bash
# Self-install any missing tool
mise(action="install", tool="ast-grep")

# Run project tasks defined in mise.toml
mise(action="run", task="test")

# Watch for changes and rerun
mise(action="watch", task="build")

# Search for available tools
mise(action="search", tool="python")

# Check for outdated tools
mise(action="outdated")

# Environment management
mise(action="env")
```

## Git Tools

| Tool | Description |
|------|-------------|
| `git_status` | Repository status (staged, unstaged, untracked) |
| `git_diff` | Show changes (supports staged, file-specific) |
| `git_add` | Stage files for commit |
| `git_commit` | Create a commit with message |
| `git_log` | View commit history (configurable count, format) |
| `git_blame` | Line-by-line authorship for a file |
| `git_branch` | List branches, show current branch |
| `git_checkout` | Switch branches, create branches, restore files |
| `git_stash` | Stash operations: push, pop, list, drop, show |

## Agent

| Tool | Description |
|------|-------------|
| `spawn_agent` | Spawn a sub-agent for delegated tasks |
| `spawn_agents` | Spawn multiple sub-agents in parallel |

## MCP Tools

Pawan connects to MCP servers configured in `pawan.toml`:

```toml
[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

MCP tools are namespaced as `mcp_<server>_<tool>` (e.g., `mcp_daedra_web_search`).

List discovered tools: `pawan mcp list`

## Edit Modes

### Standard Edit
Replace exact strings in files — works like Claude Code's Edit tool.

### Anchor Mode
Find the target line by content instead of line number. Immune to LLM line-counting errors.

```
anchor_text: "fn main()"
anchor_count: 1        # 1st occurrence (default)
new_content: "fn main() -> Result<()>"
```

### Block-Aware Insert
`insert_after` detects if the anchor line ends with `{` and skips to the matching `}` before inserting.

## Safety & Intelligence

- **Auto-install**: Missing CLI tools (ast-grep, rg, fd, sd) are auto-installed via mise on first use
- **Tiered visibility**: Only core tools in LLM prompt by default — extended tools activate on first use
- **Path normalization**: All file tools detect and correct double workspace prefix
- **Compile-gated confidence**: After writing `.rs` files, `cargo check` runs automatically
- **Iteration budget awareness**: Model warned at 3 remaining iterations
- **Token budget tracking**: Thinking vs action tokens tracked per call, visible in TUI
- **Enforceable thinking budget**: `thinking_budget > 0` disables thinking, all tokens go to action
