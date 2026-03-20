+++
title = "Tools"
+++

Pawan ships 28 built-in tools plus dynamic MCP tool discovery.

## File Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents (supports line offset/limit) |
| `write_file` | Create or overwrite a file. Auto-creates parent dirs. Path normalization detects double workspace prefix. |
| `edit_file_lines` | Precise editing with anchor-mode (find line by content, not number) |
| `edit_file` | String replacement editing (old_string → new_string) |
| `insert_after` | Block-aware insertion (skips over function/struct bodies) |
| `append_file` | Append content to end of file |
| `list_directory` | List directory contents with file metadata |

## Search Tools

| Tool | Description |
|------|-------------|
| `glob_search` | Find files by glob pattern (e.g., `**/*.rs`) |
| `grep_search` | Search file contents by regex pattern |
| `ripgrep` | Native `rg` wrapper — pattern, type filter, context, case-insensitive, invert, hidden, max-depth |
| `fd` | Native `fd` wrapper — find files by name/extension/type with max-depth and max-results |

## Code Intelligence

| Tool | Description |
|------|-------------|
| `ast_grep` | **AST-level code search and rewrite** — structural patterns via tree-sitter. `$VAR` for single-node wildcards, `$$$VAR` for variadic. Supports rust, python, js, ts, go, c, cpp, java. |

### ast-grep examples

```bash
# Find all unwrap() calls
ast_grep(action="search", pattern="$EXPR.unwrap()", lang="rust", path="src/")

# Replace unwrap() with ? operator
ast_grep(action="rewrite", pattern="$EXPR.unwrap()", rewrite="$EXPR?", lang="rust", path="src/")

# Find all function signatures
ast_grep(action="search", pattern="fn $F($$$A) { $$$ }", lang="rust", path=".")

# Rename a struct across the codebase
ast_grep(action="rewrite", pattern="OldName", rewrite="NewName", lang="rust", path="src/")
```

## Shell & System

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with configurable timeout |
| `sd` | Native `sd` wrapper — find-and-replace in files (fixed strings or regex) |
| `tree` | Native `erd` tree view — directory structure with depth and pattern filters |
| `mise` | Runtime manager — install, list, use, exec tools (any language/toolchain) |
| `zoxide` | Smart directory navigation — query, add, list paths |

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

MCP tools are namespaced as `mcp_<server>_<tool>` (e.g., `mcp_daedra_search_duckduckgo`).

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
`insert_after` detects if the anchor line ends with `{` and skips to the matching `}` before inserting. No more accidentally splitting function bodies.

## Safety Features

- **Path normalization**: All file tools detect and correct double workspace prefix (e.g., `/ws/root/ws/root/file` → `/ws/root/file`)
- **Compile-gated confidence**: After writing `.rs` files, `cargo check` runs automatically — errors injected back to the model for self-correction
- **Iteration budget awareness**: Model is warned when 3 tool iterations remain
- **Token budget tracking**: Thinking vs action tokens tracked per call, visible in TUI and CLI output
