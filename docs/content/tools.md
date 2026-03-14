+++
title = "Tools"
+++

Pawan has 17 built-in tools plus dynamic MCP tool discovery.

## File Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents (supports line offset/limit) |
| `write_file` | Create or overwrite a file |
| `edit_file` | Precise string replacement (old_string → new_string, supports replace_all) |
| `list_directory` | List directory contents with file metadata |

## Search Tools

| Tool | Description |
|------|-------------|
| `glob_search` | Find files by glob pattern (e.g., `**/*.rs`) |
| `grep_search` | Search file contents by regex pattern |

## Shell

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands with configurable timeout |

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

## MCP Tools

Pawan connects to MCP servers configured in `pawan.toml`:

```toml
[mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
```

MCP tools are namespaced as `mcp_<server>_<tool>` (e.g., `mcp_daedra_search_duckduckgo`).

List discovered tools: `pawan mcp list`
