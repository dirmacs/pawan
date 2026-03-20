//! Native CLI tool wrappers — rg, fd, sd, ag, erd
//!
//! Each tool auto-checks binary availability and provides structured output.
//! Faster than bash for search/replace tasks because they bypass shell parsing.

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;

/// Check if a binary exists in PATH
/// Check if a CLI binary is available in PATH.
fn binary_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Run a command and capture stdout+stderr
/// Execute a command and capture stdout, stderr, and success status.
async fn run_cmd(cmd: &str, args: &[&str], cwd: &std::path::Path) -> Result<(String, String, bool), String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run {}: {}", cmd, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status.success()))
}

// ─── ripgrep (rg) ───────────────────────────────────────────────────────────

/// Tool for fast text search using ripgrep
///
/// This tool provides fast recursive search through files using the rg (ripgrep)
/// command line tool. It returns structured JSON results for easy parsing.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct RipgrepTool {
    workspace_root: PathBuf,
}

impl RipgrepTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for RipgrepTool {
    fn name(&self) -> &str { "rg" }

    fn description(&self) -> &str {
        "ripgrep — blazing fast regex search across files. Returns matching lines with file paths \
         and line numbers. Use for finding code patterns, function definitions, imports, usages. \
         Much faster than bash grep. Supports --type for language filtering (rust, py, js, go)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "Path to search in (default: workspace root)" },
                "type_filter": { "type": "string", "description": "File type filter: rust, py, js, go, ts, c, cpp, java, toml, md" },
                "max_count": { "type": "integer", "description": "Max matches per file (default: 20)" },
                "context": { "type": "integer", "description": "Lines of context around each match (default: 0)" },
                "fixed_strings": { "type": "boolean", "description": "Treat pattern as literal string, not regex" },
                "case_insensitive": { "type": "boolean", "description": "Case insensitive search (default: false)" },
                "invert": { "type": "boolean", "description": "Invert match: show lines that do NOT match (default: false)" },
                "hidden": { "type": "boolean", "description": "Search hidden files and directories (default: false)" },
                "max_depth": { "type": "integer", "description": "Max directory depth to search (default: unlimited)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        if !binary_exists("rg") {
            return Err(crate::PawanError::Tool("rg not found. Install: cargo install ripgrep".into()));
        }
        let pattern = args["pattern"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let max_count = args["max_count"].as_u64().unwrap_or(20);
        let context = args["context"].as_u64().unwrap_or(0);

        let max_count_str = max_count.to_string();
        let ctx_str = context.to_string();
        let mut cmd_args = vec![
            "--line-number", "--no-heading", "--color", "never",
            "--max-count", &max_count_str,
        ];
        if context > 0 {
            cmd_args.extend_from_slice(&["--context", &ctx_str]);
        }
        if let Some(t) = args["type_filter"].as_str() {
            cmd_args.extend_from_slice(&["--type", t]);
        }
        if args["fixed_strings"].as_bool().unwrap_or(false) {
            cmd_args.push("--fixed-strings");
        }
        if args["case_insensitive"].as_bool().unwrap_or(false) {
            cmd_args.push("-i");
        }
        if args["invert"].as_bool().unwrap_or(false) {
            cmd_args.push("--invert-match");
        }
        if args["hidden"].as_bool().unwrap_or(false) {
            cmd_args.push("--hidden");
        }
        let depth_str = args["max_depth"].as_u64().map(|d| d.to_string());
        if let Some(ref ds) = depth_str {
            cmd_args.push("--max-depth");
            cmd_args.push(ds);
        }
        cmd_args.push(pattern);
        cmd_args.push(search_path);

        let cwd = if std::path::Path::new(search_path).is_absolute() {
            std::path::PathBuf::from("/")
        } else {
            self.workspace_root.clone()
        };

        let (stdout, stderr, success) = run_cmd("rg", &cmd_args, &cwd).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        let match_count = stdout.lines().filter(|l| !l.is_empty()).count();

        Ok(json!({
            "matches": stdout.lines().take(100).collect::<Vec<_>>().join("\n"),
            "match_count": match_count,
            "success": success || match_count > 0,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── fd (fast find) ─────────────────────────────────────────────────────────

/// Tool for fast file search using fd
///
/// This tool provides fast file and directory search using the fd command line
/// tool. It's an alternative to find that's faster and more user-friendly.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct FdTool {
    workspace_root: PathBuf,
}

impl FdTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for FdTool {
    fn name(&self) -> &str { "fd" }

    fn description(&self) -> &str {
        "fd — fast file finder. Finds files and directories by name pattern. \
         Much faster than bash find. Use for locating files, exploring project structure."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Search pattern (regex by default)" },
                "path": { "type": "string", "description": "Directory to search in (default: workspace root)" },
                "extension": { "type": "string", "description": "Filter by extension: rs, py, js, toml, md" },
                "type_filter": { "type": "string", "description": "f=file, d=directory, l=symlink" },
                "max_depth": { "type": "integer", "description": "Max directory depth" },
                "max_results": { "type": "integer", "description": "Max results to return (default: 50, prevents context flooding)" },
                "hidden": { "type": "boolean", "description": "Include hidden files (default: false)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        if !binary_exists("fd") {
            return Err(crate::PawanError::Tool("fd not found. Install: cargo install fd-find".into()));
        }
        let pattern = args["pattern"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;

        let mut cmd_args: Vec<String> = vec!["--color".into(), "never".into()];
        if let Some(ext) = args["extension"].as_str() {
            cmd_args.push("-e".into()); cmd_args.push(ext.into());
        }
        if let Some(t) = args["type_filter"].as_str() {
            cmd_args.push("-t".into()); cmd_args.push(t.into());
        }
        if let Some(d) = args["max_depth"].as_u64() {
            cmd_args.push("--max-depth".into()); cmd_args.push(d.to_string());
        }
        if args["hidden"].as_bool().unwrap_or(false) {
            cmd_args.push("--hidden".into());
        }
        cmd_args.push(pattern.into());
        if let Some(p) = args["path"].as_str() {
            cmd_args.push(p.into());
        }

        let cmd_args_ref: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, _) = run_cmd("fd", &cmd_args_ref, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;
        let all_files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
        let total = all_files.len();
        let files: Vec<&str> = all_files.into_iter().take(max_results).collect();
        let truncated = total > max_results;

        Ok(json!({
            "files": files,
            "count": files.len(),
            "total_found": total,
            "truncated": truncated,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── sd (fast sed) ──────────────────────────────────────────────────────────

/// Tool for fast text replacement using sd
///
/// This tool provides fast text replacement using the sd command line tool.
/// It's an alternative to sed that's more intuitive and faster.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct SdTool {
    workspace_root: PathBuf,
}

impl SdTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for SdTool {
    fn name(&self) -> &str { "sd" }

    fn description(&self) -> &str {
        "sd — fast find-and-replace across files. Like sed but simpler syntax and faster. \
         Use for bulk renaming, refactoring imports, changing patterns across entire codebase. \
         Modifies files in-place."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "find": { "type": "string", "description": "Pattern to find (regex)" },
                "replace": { "type": "string", "description": "Replacement string" },
                "path": { "type": "string", "description": "File or directory to process" },
                "fixed_strings": { "type": "boolean", "description": "Treat find as literal, not regex (default: false)" }
            },
            "required": ["find", "replace", "path"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        if !binary_exists("sd") {
            return Err(crate::PawanError::Tool("sd not found. Install: cargo install sd".into()));
        }
        let find = args["find"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("find required".into()))?;
        let replace = args["replace"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("replace required".into()))?;
        let path = args["path"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("path required".into()))?;

        let mut cmd_args = vec![];
        if args["fixed_strings"].as_bool().unwrap_or(false) {
            cmd_args.push("-F");
        }
        cmd_args.extend_from_slice(&[find, replace, path]);

        let (stdout, stderr, success) = run_cmd("sd", &cmd_args, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        Ok(json!({
            "success": success,
            "output": stdout,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── erdtree (erd) ──────────────────────────────────────────────────────────

pub struct ErdTool {
    workspace_root: PathBuf,
}

impl ErdTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for ErdTool {
    fn name(&self) -> &str { "tree" }

    fn description(&self) -> &str {
        "erdtree (erd) — fast filesystem tree with disk usage, file counts, and metadata. \
         Use to map project structure, find large files/dirs, count lines of code, \
         audit disk usage, or get a flat file listing. Much faster than find + du."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Root directory (default: workspace root)" },
                "depth": { "type": "integer", "description": "Max traversal depth (default: 3)" },
                "pattern": { "type": "string", "description": "Filter by glob pattern (e.g. '*.rs', 'Cargo*')" },
                "sort": {
                    "type": "string",
                    "enum": ["name", "size", "type"],
                    "description": "Sort entries by name, size, or file type (default: name)"
                },
                "disk_usage": {
                    "type": "string",
                    "enum": ["physical", "logical", "line", "word", "block"],
                    "description": "Disk usage mode: physical (bytes on disk), logical (file size), line (line count), word (word count). Default: physical."
                },
                "layout": {
                    "type": "string",
                    "enum": ["regular", "inverted", "flat", "iflat"],
                    "description": "Output layout: regular (root at bottom), inverted (root at top), flat (paths only), iflat (flat + root at top). Default: inverted."
                },
                "long": { "type": "boolean", "description": "Show extended metadata: permissions, owner, group, timestamps (default: false)" },
                "hidden": { "type": "boolean", "description": "Show hidden (dot) files (default: false)" },
                "dirs_only": { "type": "boolean", "description": "Only show directories, not files (default: false)" },
                "human": { "type": "boolean", "description": "Human-readable sizes like 4.2M instead of bytes (default: true)" },
                "icons": { "type": "boolean", "description": "Show file type icons (default: false)" },
                "no_ignore": { "type": "boolean", "description": "Don't respect .gitignore (default: false)" },
                "suppress_size": { "type": "boolean", "description": "Hide disk usage column (default: false)" }
            }
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        if !binary_exists("erd") {
            // Fallback: fd-based flat listing
            let path = args["path"].as_str().unwrap_or(".");
            let depth = args["depth"].as_u64().unwrap_or(3).to_string();
            let cmd_args = vec![".", path, "--max-depth", &depth, "--color", "never"];
            let (stdout, _, _) = run_cmd("fd", &cmd_args, &self.workspace_root).await
                .unwrap_or(("(fd not available — install erd: cargo install erdtree)".into(), String::new(), false));
            return Ok(json!({ "tree": stdout, "tool": "fd-fallback" }));
        }

        let path = args["path"].as_str().unwrap_or(".");
        let depth_str = args["depth"].as_u64().unwrap_or(3).to_string();

        let mut cmd_args: Vec<String> = vec![
            "--level".into(), depth_str,
            "--no-config".into(),
            "--color".into(), "none".into(),
        ];

        // Disk usage mode
        if let Some(du) = args["disk_usage"].as_str() {
            cmd_args.extend(["--disk-usage".into(), du.into()]);
        }

        // Layout
        let layout = args["layout"].as_str().unwrap_or("inverted");
        cmd_args.extend(["--layout".into(), layout.into()]);

        // Sort
        if let Some(sort) = args["sort"].as_str() {
            cmd_args.extend(["--sort".into(), sort.into()]);
        }

        // Pattern filter
        if let Some(p) = args["pattern"].as_str() {
            cmd_args.extend(["--pattern".into(), p.into()]);
        }

        // Boolean flags
        if args["long"].as_bool().unwrap_or(false) { cmd_args.push("--long".into()); }
        if args["hidden"].as_bool().unwrap_or(false) { cmd_args.push("--hidden".into()); }
        if args["dirs_only"].as_bool().unwrap_or(false) { cmd_args.push("--dirs-only".into()); }
        if args["human"].as_bool().unwrap_or(true) { cmd_args.push("--human".into()); }
        if args["icons"].as_bool().unwrap_or(false) { cmd_args.push("--icons".into()); }
        if args["no_ignore"].as_bool().unwrap_or(false) { cmd_args.push("--no-ignore".into()); }
        if args["suppress_size"].as_bool().unwrap_or(false) { cmd_args.push("--suppress-size".into()); }

        cmd_args.push(path.into());

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("erd", &cmd_refs, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        Ok(json!({
            "success": success,
            "tree": stdout,
            "tool": "erd",
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── grep_search (rg wrapper for structured output) ─────────────────────────

pub struct GrepSearchTool {
    workspace_root: PathBuf,
}

impl GrepSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GrepSearchTool {
    fn name(&self) -> &str { "grep_search" }

    fn description(&self) -> &str {
        "Search for a pattern in files using ripgrep. Returns file paths and matching lines. \
         Prefer this over bash grep — it's faster and respects .gitignore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Search pattern (regex)" },
                "path": { "type": "string", "description": "Path to search (default: workspace)" },
                "include": { "type": "string", "description": "Glob to include (e.g. '*.rs')" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str().unwrap_or(".");

        let mut cmd_args = vec!["--line-number", "--no-heading", "--color", "never", "--max-count", "30"];
        let include;
        if let Some(glob) = args["include"].as_str() {
            include = format!("--glob={}", glob);
            cmd_args.push(&include);
        }
        cmd_args.push(pattern);
        cmd_args.push(path);

        let cwd = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from("/")
        } else {
            self.workspace_root.clone()
        };

        let (stdout, _, _) = run_cmd("rg", &cmd_args, &cwd).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).take(50).collect();

        Ok(json!({
            "results": lines.join("\n"),
            "count": lines.len()
        }))
    }
}

// ─── glob_search (fd wrapper) ───────────────────────────────────────────────

pub struct GlobSearchTool {
    workspace_root: PathBuf,
}

impl GlobSearchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str { "glob_search" }

    fn description(&self) -> &str {
        "Find files by glob pattern. Returns list of matching file paths. \
         Uses fd under the hood — fast, respects .gitignore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g. '*.rs', 'test_*')" },
                "path": { "type": "string", "description": "Directory to search (default: workspace)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str().unwrap_or(".");

        let cmd_args = vec!["--glob", pattern, "--color", "never", path];
        let (stdout, _, _) = run_cmd("fd", &cmd_args, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        let files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

        Ok(json!({
            "files": files,
            "count": files.len()
        }))
    }
}

// ─── mise (universal tool installer) ────────────────────────────────────────

pub struct MiseTool {
    workspace_root: PathBuf,
}

impl MiseTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for MiseTool {
    fn name(&self) -> &str { "mise" }

    fn description(&self) -> &str {
        "mise — polyglot tool manager, environment manager, and task runner. Replaces asdf, nvm, \
         pyenv, direnv, make, and npm scripts. Three powers: (1) install/manage any dev tool or \
         language runtime, (2) manage per-project env vars, (3) run/watch project tasks. \
         Pawan should use this to self-install any missing CLI tool (erd, ast-grep, fd, rg, etc)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "install", "uninstall", "upgrade", "list", "use", "search",
                        "exec", "run", "tasks", "env", "outdated", "prune",
                        "doctor", "self-update", "trust", "watch"
                    ],
                    "description": "Tool management: install, uninstall, upgrade, list, use, search, outdated, prune. \
                                    Execution: exec (run with tool env), run (run a task), watch (rerun task on file change). \
                                    Environment: env (show/set env vars). Tasks: tasks (list/manage tasks). \
                                    Maintenance: doctor, self-update, trust, prune."
                },
                "tool": {
                    "type": "string",
                    "description": "Tool name with optional version. Examples: 'erdtree', 'node@22', 'python@3.12', \
                                    'ast-grep', 'ripgrep', 'fd', 'sd', 'bat', 'delta', 'jq', 'yq', 'go', 'bun', 'deno'"
                },
                "task": {
                    "type": "string",
                    "description": "Task name for run/watch/tasks actions (defined in mise.toml or .mise/tasks/)"
                },
                "args": {
                    "type": "string",
                    "description": "Additional arguments (space-separated). For exec: command to run. For run: task args."
                },
                "global": {
                    "type": "boolean",
                    "description": "Apply globally (--global flag) instead of project-local. Default: false."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let mise_bin = if binary_exists("mise") {
            "mise".to_string()
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            let local = format!("{}/.local/bin/mise", home);
            if std::path::Path::new(&local).exists() { local } else {
                return Err(crate::PawanError::Tool(
                    "mise not found. Install: curl https://mise.run | sh".into()
                ));
            }
        };

        let action = args["action"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required".into()))?;
        let global = args["global"].as_bool().unwrap_or(false);

        let cmd_args: Vec<String> = match action {
            "install" => {
                let tool = args["tool"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for install".into()))?;
                vec!["install".into(), tool.into(), "-y".into()]
            }
            "uninstall" => {
                let tool = args["tool"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for uninstall".into()))?;
                vec!["uninstall".into(), tool.into()]
            }
            "upgrade" => {
                let mut v = vec!["upgrade".into()];
                if let Some(tool) = args["tool"].as_str() { v.push(tool.into()); }
                v
            }
            "list" => vec!["ls".into()],
            "search" => {
                let tool = args["tool"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for search".into()))?;
                vec!["registry".into(), tool.into()]
            }
            "use" => {
                let tool = args["tool"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for use".into()))?;
                let mut v = vec!["use".into()];
                if global { v.push("--global".into()); }
                v.push(tool.into());
                v
            }
            "outdated" => {
                let mut v = vec!["outdated".into()];
                if let Some(tool) = args["tool"].as_str() { v.push(tool.into()); }
                v
            }
            "prune" => {
                let mut v = vec!["prune".into(), "-y".into()];
                if let Some(tool) = args["tool"].as_str() { v.push(tool.into()); }
                v
            }
            "exec" => {
                let tool = args["tool"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("tool required for exec".into()))?;
                let extra = args["args"].as_str().unwrap_or("");
                let mut v = vec!["exec".into(), tool.into(), "--".into()];
                if !extra.is_empty() {
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "run" => {
                let task = args["task"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("task required for run".into()))?;
                let mut v = vec!["run".into(), task.into()];
                if let Some(extra) = args["args"].as_str() {
                    v.push("--".into());
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "watch" => {
                let task = args["task"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("task required for watch".into()))?;
                let mut v = vec!["watch".into(), task.into()];
                if let Some(extra) = args["args"].as_str() {
                    v.push("--".into());
                    v.extend(extra.split_whitespace().map(|s| s.to_string()));
                }
                v
            }
            "tasks" => vec!["tasks".into(), "ls".into()],
            "env" => vec!["env".into()],
            "doctor" => vec!["doctor".into()],
            "self-update" => vec!["self-update".into(), "-y".into()],
            "trust" => {
                let mut v = vec!["trust".into()];
                if let Some(extra) = args["args"].as_str() { v.push(extra.into()); }
                v
            }
            _ => return Err(crate::PawanError::Tool(
                format!("Unknown action: {action}. See tool description for available actions.")
            )),
        };

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd(&mise_bin, &cmd_refs, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        Ok(json!({
            "success": success,
            "action": action,
            "output": stdout,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── zoxide (smart cd) ─────────────────────────────────────────────────────

pub struct ZoxideTool {
    workspace_root: PathBuf,
}

impl ZoxideTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for ZoxideTool {
    fn name(&self) -> &str { "z" }

    fn description(&self) -> &str {
        "zoxide — smart directory jumper. Learns from your cd history. \
         Use 'query' to find a directory by fuzzy match (e.g. 'pawan' finds /opt/pawan). \
         Use 'add' to teach it a new path. Use 'list' to see known paths."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "query, add, or list" },
                "path": { "type": "string", "description": "Path or search term" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let action = args["action"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required (query/add/list)".into()))?;

        let cmd_args: Vec<String> = match action {
            "query" => {
                let path = args["path"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("path/search term required for query".into()))?;
                vec!["query".into(), path.into()]
            }
            "add" => {
                let path = args["path"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("path required for add".into()))?;
                vec!["add".into(), path.into()]
            }
            "list" => vec!["query".into(), "--list".into()],
            _ => return Err(crate::PawanError::Tool(format!("Unknown action: {}. Use query/add/list", action))),
        };

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("zoxide", &cmd_refs, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        Ok(json!({
            "success": success,
            "result": stdout.trim(),
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}

// ─── ast-grep ────────────────────────────────────────────────────────────────

pub struct AstGrepTool {
    workspace_root: PathBuf,
}

impl AstGrepTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for AstGrepTool {
    fn name(&self) -> &str { "ast_grep" }

    fn description(&self) -> &str {
        "ast-grep — structural code search and rewrite using AST patterns. \
         Unlike regex, this matches code by syntax tree structure. Use $NAME for \
         single-node wildcards, $$$ARGS for variadic (multiple nodes). \
         Actions: 'search' finds matches, 'rewrite' transforms them in-place. \
         Examples: pattern='fn $NAME($$$ARGS)' finds all functions. \
         pattern='$EXPR.unwrap()' rewrite='$EXPR?' replaces unwrap with ?. \
         Supports: rust, python, javascript, typescript, go, c, cpp, java."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "rewrite"],
                    "description": "search: find matching code. rewrite: transform matching code in-place."
                },
                "pattern": {
                    "type": "string",
                    "description": "AST pattern to match. Use $VAR for wildcards, $$$VAR for variadic. e.g. 'fn $NAME($$$ARGS) -> $RET { $$$ }'"
                },
                "rewrite": {
                    "type": "string",
                    "description": "Replacement pattern (only for action=rewrite). Use captured $VARs. e.g. '$EXPR?'"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search/rewrite"
                },
                "lang": {
                    "type": "string",
                    "description": "Language: rust, python, javascript, typescript, go, c, cpp, java (default: auto-detect)"
                }
            },
            "required": ["action", "pattern", "path"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        if !binary_exists("ast-grep") {
            return Err(crate::PawanError::Tool(
                "ast-grep not found. Install: cargo install ast-grep".into(),
            ));
        }

        let action = args["action"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("action required (search or rewrite)".into()))?;
        let pattern = args["pattern"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str()
            .ok_or_else(|| crate::PawanError::Tool("path required".into()))?;

        let mut cmd_args: Vec<String> = vec!["run".into()];

        // Language
        if let Some(lang) = args["lang"].as_str() {
            cmd_args.push("--lang".into());
            cmd_args.push(lang.into());
        }

        // Pattern
        cmd_args.push("--pattern".into());
        cmd_args.push(pattern.into());

        match action {
            "search" => {
                // Search mode: just find and report matches
                cmd_args.push(path.into());
            }
            "rewrite" => {
                let rewrite = args["rewrite"].as_str()
                    .ok_or_else(|| crate::PawanError::Tool("rewrite pattern required for action=rewrite".into()))?;
                cmd_args.push("--rewrite".into());
                cmd_args.push(rewrite.into());
                cmd_args.push("--update-all".into());
                cmd_args.push(path.into());
            }
            _ => {
                return Err(crate::PawanError::Tool(
                    format!("Unknown action: {}. Use 'search' or 'rewrite'", action),
                ));
            }
        }

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("ast-grep", &cmd_refs, &self.workspace_root).await
            .map_err(|e| crate::PawanError::Tool(e))?;

        // Count matches from output
        let match_count = stdout.lines()
            .filter(|l| l.starts_with('/') || l.contains("│"))
            .count();

        Ok(json!({
            "success": success,
            "action": action,
            "matches": match_count,
            "output": stdout,
            "stderr": if stderr.is_empty() { None::<String> } else { Some(stderr) }
        }))
    }
}
