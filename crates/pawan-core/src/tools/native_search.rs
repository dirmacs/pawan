//! Native CLI search tool wrappers — rg, fd, sd, erd.
//!
//! Helper functions and tool structs that wrap CLI binaries for search tasks.
//! Also includes structured-output wrappers: GrepSearchTool (rg) and GlobSearchTool (fd).

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;

// ─── helpers ────────────────────────────────────────────────────────────────

/// Check if a CLI binary is available in PATH.
pub(crate) fn binary_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Map tool binary names to their mise package names for auto-install.
pub(crate) fn mise_package_name(binary: &str) -> &str {
    match binary {
        "erd" => "erdtree",
        "sg" | "ast-grep" => "ast-grep",
        "rg" => "ripgrep",
        "fd" => "fd",
        "sd" => "sd",
        "bat" => "bat",
        "delta" => "delta",
        "jq" => "jq",
        "yq" => "yq",
        other => other,
    }
}

/// Try to auto-install a missing tool via mise. Returns true if install succeeded.
pub(crate) async fn auto_install(binary: &str, cwd: &std::path::Path) -> bool {
    let mise_bin = if binary_exists("mise") {
        "mise".to_string()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let local = format!("{}/.local/bin/mise", home);
        if std::path::Path::new(&local).exists() {
            local
        } else {
            return false;
        }
    };

    let pkg = mise_package_name(binary);
    tracing::info!(
        binary = binary,
        package = pkg,
        "Auto-installing missing tool via mise"
    );

    let result = tokio::process::Command::new(&mise_bin)
        .args(["install", pkg, "-y"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            // Also run `mise use --global` to make it available
            let _ = tokio::process::Command::new(&mise_bin)
                .args(["use", "--global", pkg])
                .current_dir(cwd)
                .output()
                .await;
            tracing::info!(binary = binary, "Auto-install succeeded");
            true
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(binary = binary, stderr = %stderr, "Auto-install failed");
            false
        }
        Err(e) => {
            tracing::warn!(binary = binary, error = %e, "Auto-install failed to run mise");
            false
        }
    }
}

/// Ensure a binary is available, auto-installing via mise if needed.
pub(crate) async fn ensure_binary(
    name: &str,
    cwd: &std::path::Path,
) -> Result<(), crate::PawanError> {
    if binary_exists(name) {
        return Ok(());
    }
    if auto_install(name, cwd).await && binary_exists(name) {
        return Ok(());
    }
    Err(crate::PawanError::Tool(format!(
        "{} not found and auto-install failed. Install manually: mise install {}",
        name,
        mise_package_name(name)
    )))
}

/// Execute a command and capture stdout, stderr, and success status.
pub(crate) async fn run_cmd(
    cmd: &str,
    args: &[&str],
    cwd: &std::path::Path,
) -> Result<(String, String, bool), String> {
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

/// Tool for fast text search using ripgrep.
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
    fn name(&self) -> &str {
        "rg"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Regex pattern to search for")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Path to search in (default: workspace root)")
                    .build(),
            )
            .parameter(
                Parameter::builder("type_filter")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("File type filter: rust, py, js, go, ts, c, cpp, java, toml, md")
                    .build(),
            )
            .parameter(
                Parameter::builder("max_count")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max matches per file (default: 20)")
                    .build(),
            )
            .parameter(
                Parameter::builder("context")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Lines of context around each match (default: 0)")
                    .build(),
            )
            .parameter(
                Parameter::builder("fixed_strings")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Treat pattern as literal string, not regex")
                    .build(),
            )
            .parameter(
                Parameter::builder("case_insensitive")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Case insensitive search (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("invert")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Invert match: show lines that do NOT match (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("hidden")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Search hidden files and directories (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("max_depth")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max directory depth to search (default: unlimited)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_binary("rg", &self.workspace_root).await?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let max_count = args["max_count"].as_u64().unwrap_or(20);
        let context = args["context"].as_u64().unwrap_or(0);

        let max_count_str = max_count.to_string();
        let ctx_str = context.to_string();
        let mut cmd_args = vec![
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "--max-count",
            &max_count_str,
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

        let (stdout, stderr, success) = run_cmd("rg", &cmd_args, &cwd)
            .await
            .map_err(crate::PawanError::Tool)?;

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

/// Tool for fast file search using fd.
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
    fn name(&self) -> &str {
        "fd"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Search pattern (regex by default)")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Directory to search in (default: workspace root)")
                    .build(),
            )
            .parameter(
                Parameter::builder("extension")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Filter by extension: rs, py, js, toml, md")
                    .build(),
            )
            .parameter(
                Parameter::builder("type_filter")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("f=file, d=directory, l=symlink")
                    .build(),
            )
            .parameter(
                Parameter::builder("max_depth")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max directory depth")
                    .build(),
            )
            .parameter(
                Parameter::builder("max_results")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max results to return (default: 50, prevents context flooding)")
                    .build(),
            )
            .parameter(
                Parameter::builder("hidden")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Include hidden files (default: false)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_binary("fd", &self.workspace_root).await?;
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;

        let mut cmd_args: Vec<String> = vec!["--color".into(), "never".into()];
        if let Some(ext) = args["extension"].as_str() {
            cmd_args.push("-e".into());
            cmd_args.push(ext.into());
        }
        if let Some(t) = args["type_filter"].as_str() {
            cmd_args.push("-t".into());
            cmd_args.push(t.into());
        }
        if let Some(d) = args["max_depth"].as_u64() {
            cmd_args.push("--max-depth".into());
            cmd_args.push(d.to_string());
        }
        if args["hidden"].as_bool().unwrap_or(false) {
            cmd_args.push("--hidden".into());
        }
        cmd_args.push(pattern.into());
        if let Some(p) = args["path"].as_str() {
            cmd_args.push(p.into());
        }

        let cmd_args_ref: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, _) = run_cmd("fd", &cmd_args_ref, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

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

/// Tool for fast text replacement using sd.
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
    fn name(&self) -> &str {
        "sd"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("find")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Pattern to find (regex)")
                    .build(),
            )
            .parameter(
                Parameter::builder("replace")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Replacement string")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("File or directory to process")
                    .build(),
            )
            .parameter(
                Parameter::builder("fixed_strings")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Treat find as literal, not regex (default: false)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        ensure_binary("sd", &self.workspace_root).await?;
        let find = args["find"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("find required".into()))?;
        let replace = args["replace"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("replace required".into()))?;
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("path required".into()))?;

        let mut cmd_args = vec![];
        if args["fixed_strings"].as_bool().unwrap_or(false) {
            cmd_args.push("-F");
        }
        cmd_args.extend_from_slice(&[find, replace, path]);

        let (stdout, stderr, success) = run_cmd("sd", &cmd_args, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

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
    fn name(&self) -> &str {
        "tree"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Root directory (default: workspace root)")
                    .build(),
            )
            .parameter(
                Parameter::builder("depth")
                    .param_type(ParameterType::Integer)
                    .required(false)
                    .description("Max traversal depth (default: 3)")
                    .build(),
            )
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Filter by glob pattern (e.g. '*.rs', 'Cargo*')")
                    .build(),
            )
            .parameter(
                Parameter::builder("sort")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Sort entries by name, size, or file type (default: name)")
                    .build(),
            )
            .parameter(
                Parameter::builder("disk_usage")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Disk usage mode: physical (bytes on disk), logical (file size), line (line count), word (word count). Default: physical.")
                    .build(),
            )
            .parameter(
                Parameter::builder("layout")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Output layout: regular (root at bottom), inverted (root at top), flat (paths only), iflat (flat + root at top). Default: inverted.")
                    .build(),
            )
            .parameter(
                Parameter::builder("long")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Show extended metadata: permissions, owner, group, timestamps (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("hidden")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Show hidden (dot) files (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("dirs_only")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Only show directories, not files (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("human")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Human-readable sizes like 4.2M instead of bytes (default: true)")
                    .build(),
            )
            .parameter(
                Parameter::builder("icons")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Show file type icons (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("no_ignore")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Don't respect .gitignore (default: false)")
                    .build(),
            )
            .parameter(
                Parameter::builder("suppress_size")
                    .param_type(ParameterType::Boolean)
                    .required(false)
                    .description("Hide disk usage column (default: false)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        // Auto-install erd if missing; fall back to fd if mise unavailable
        if !binary_exists("erd")
            && (!auto_install("erd", &self.workspace_root).await || !binary_exists("erd"))
        {
            let path = args["path"].as_str().unwrap_or(".");
            let depth = args["depth"].as_u64().unwrap_or(3).to_string();
            let cmd_args = vec![".", path, "--max-depth", &depth, "--color", "never"];
            let (stdout, _, _) = run_cmd("fd", &cmd_args, &self.workspace_root)
                .await
                .unwrap_or(("(erd and fd not available)".into(), String::new(), false));
            return Ok(json!({ "tree": stdout, "tool": "fd-fallback" }));
        }

        let path = args["path"].as_str().unwrap_or(".");
        let depth_str = args["depth"].as_u64().unwrap_or(3).to_string();

        let mut cmd_args: Vec<String> = vec![
            "--level".into(),
            depth_str,
            "--no-config".into(),
            "--color".into(),
            "none".into(),
        ];

        if let Some(du) = args["disk_usage"].as_str() {
            cmd_args.extend(["--disk-usage".into(), du.into()]);
        }

        let layout = args["layout"].as_str().unwrap_or("inverted");
        cmd_args.extend(["--layout".into(), layout.into()]);

        if let Some(sort) = args["sort"].as_str() {
            cmd_args.extend(["--sort".into(), sort.into()]);
        }

        if let Some(p) = args["pattern"].as_str() {
            cmd_args.extend(["--pattern".into(), p.into()]);
        }

        if args["long"].as_bool().unwrap_or(false) {
            cmd_args.push("--long".into());
        }
        if args["hidden"].as_bool().unwrap_or(false) {
            cmd_args.push("--hidden".into());
        }
        if args["dirs_only"].as_bool().unwrap_or(false) {
            cmd_args.push("--dirs-only".into());
        }
        if args["human"].as_bool().unwrap_or(true) {
            cmd_args.push("--human".into());
        }
        if args["icons"].as_bool().unwrap_or(false) {
            cmd_args.push("--icons".into());
        }
        if args["no_ignore"].as_bool().unwrap_or(false) {
            cmd_args.push("--no-ignore".into());
        }
        if args["suppress_size"].as_bool().unwrap_or(false) {
            cmd_args.push("--suppress-size".into());
        }

        cmd_args.push(path.into());

        let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let (stdout, stderr, success) = run_cmd("erd", &cmd_refs, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

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
    fn name(&self) -> &str {
        "grep_search"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Search pattern (regex)")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Path to search (default: workspace)")
                    .build(),
            )
            .parameter(
                Parameter::builder("include")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Glob to include (e.g. '*.rs')")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str().unwrap_or(".");

        let mut cmd_args = vec![
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "--max-count",
            "30",
        ];
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

        let (stdout, _, _) = run_cmd("rg", &cmd_args, &cwd)
            .await
            .map_err(crate::PawanError::Tool)?;

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
    fn name(&self) -> &str {
        "glob_search"
    }

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

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder(self.name())
            .description(self.description())
            .parameter(
                Parameter::builder("pattern")
                    .param_type(ParameterType::String)
                    .required(true)
                    .description("Glob pattern (e.g. '*.rs', 'test_*')")
                    .build(),
            )
            .parameter(
                Parameter::builder("path")
                    .param_type(ParameterType::String)
                    .required(false)
                    .description("Directory to search (default: workspace)")
                    .build(),
            )
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("pattern required".into()))?;
        let path = args["path"].as_str().unwrap_or(".");

        let cmd_args = vec!["--glob", pattern, "--color", "never", path];
        let (stdout, _, _) = run_cmd("fd", &cmd_args, &self.workspace_root)
            .await
            .map_err(crate::PawanError::Tool)?;

        let files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

        Ok(json!({
            "files": files,
            "count": files.len()
        }))
    }
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_binary_exists_cargo() {
        assert!(binary_exists("cargo"));
    }

    #[test]
    fn test_binary_exists_nonexistent() {
        assert!(!binary_exists("nonexistent_binary_xyz_123"));
    }

    #[test]
    fn test_mise_package_name_mapping() {
        assert_eq!(mise_package_name("rg"), "ripgrep");
        assert_eq!(mise_package_name("fd"), "fd");
        assert_eq!(mise_package_name("sg"), "ast-grep");
        assert_eq!(mise_package_name("erd"), "erdtree");
        assert_eq!(mise_package_name("unknown"), "unknown");
    }

    #[test]
    fn test_mise_package_name_all_aliases() {
        assert_eq!(mise_package_name("ast-grep"), "ast-grep");
        assert_eq!(mise_package_name("bat"), "bat");
        assert_eq!(mise_package_name("delta"), "delta");
        assert_eq!(mise_package_name("jq"), "jq");
        assert_eq!(mise_package_name("yq"), "yq");
    }

    #[test]
    fn test_mise_package_name_is_case_sensitive() {
        assert_eq!(mise_package_name("RG"), "RG");
        assert_eq!(mise_package_name("Fd"), "Fd");
        assert_eq!(mise_package_name("AST-GREP"), "AST-GREP");
    }

    #[test]
    fn test_mise_package_name_passes_through_arbitrary_names() {
        assert_eq!(mise_package_name("foo"), "foo");
        assert_eq!(mise_package_name(""), "");
        assert_eq!(
            mise_package_name("some-random-tool_v2"),
            "some-random-tool_v2"
        );
    }

    #[tokio::test]
    async fn test_rg_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        assert_eq!(tool.name(), "rg");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("pattern")));
    }

    #[tokio::test]
    async fn test_fd_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = FdTool::new(tmp.path().into());
        assert_eq!(tool.name(), "fd");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("pattern")));
    }

    #[tokio::test]
    async fn test_sd_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = SdTool::new(tmp.path().into());
        assert_eq!(tool.name(), "sd");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("find")));
        assert!(required.contains(&serde_json::json!("replace")));
    }

    #[tokio::test]
    async fn test_erd_tool_schema() {
        let tmp = TempDir::new().unwrap();
        let tool = ErdTool::new(tmp.path().to_path_buf());
        assert_eq!(tool.name(), "tree");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["disk_usage"].is_object());
        assert!(schema["properties"]["layout"].is_object());
    }

    #[tokio::test]
    async fn test_native_glob_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        assert_eq!(tool.name(), "glob_search");
        let schema = tool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("pattern")));
    }

    #[tokio::test]
    async fn test_native_grep_tool_basics() {
        let tmp = TempDir::new().unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        assert_eq!(tool.name(), "grep_search");
        let schema = tool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("pattern")));
    }

    #[tokio::test]
    async fn test_tree_tool_runs_without_crash() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.rs"), "code").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "text").unwrap();

        let tool = ErdTool::new(tmp.path().into());
        let result = tool.execute(serde_json::json!({})).await;
        assert!(
            result.is_ok(),
            "tree tool should work with fallback: {:?}",
            result.err()
        );
        let val = result.unwrap();
        assert!(
            val["tree"].is_string() || val["output"].is_string(),
            "Should produce tree output"
        );
    }

    #[tokio::test]
    async fn test_grep_search_finds_pattern_in_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("alpha.rs"),
            "fn main() {\n    println!(\"unique_marker_abc\");\n}\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("beta.rs"), "fn unrelated() {}\n").unwrap();

        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({ "pattern": "unique_marker_abc" }))
            .await
            .unwrap();

        let count = result["count"].as_u64().unwrap();
        assert!(count >= 1, "should find at least one match, got {}", count);
        let results = result["results"].as_str().unwrap();
        assert!(results.contains("unique_marker_abc"));
        assert!(results.contains("alpha.rs"));
    }

    #[tokio::test]
    async fn test_grep_search_returns_empty_on_no_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("x.rs"), "fn main() {}\n").unwrap();

        let tool = GrepSearchTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({ "pattern": "definitely_not_in_any_file_9f8e7d6c" }))
            .await
            .unwrap();

        assert_eq!(result["count"], 0);
        assert_eq!(result["results"].as_str().unwrap(), "");
    }

    #[tokio::test]
    async fn test_glob_search_finds_files_by_extension() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("one.rs"), "").unwrap();
        std::fs::write(tmp.path().join("two.rs"), "").unwrap();
        std::fs::write(tmp.path().join("ignored.txt"), "").unwrap();

        let tool = GlobSearchTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({ "pattern": "*.rs" }))
            .await
            .unwrap();

        let debug = format!("{}", result);
        assert!(
            debug.contains("one.rs") || debug.contains("two.rs"),
            "should find at least one .rs file, got: {}",
            debug
        );
        assert!(
            !debug.contains("ignored.txt"),
            "should not match .txt, got: {}",
            debug
        );
    }

    #[tokio::test]
    async fn test_ripgrep_case_insensitive_flag() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("x.txt"),
            "Hello World\nhello world\nHELLO WORLD\n",
        )
        .unwrap();

        let tool = RipgrepTool::new(tmp.path().into());

        let case_sensitive = tool
            .execute(serde_json::json!({ "pattern": "hello world" }))
            .await
            .unwrap();
        let cs_debug = format!("{}", case_sensitive);

        let case_insensitive = tool
            .execute(serde_json::json!({ "pattern": "hello world", "case_insensitive": true }))
            .await
            .unwrap();
        let ci_debug = format!("{}", case_insensitive);

        assert!(
            ci_debug.len() > cs_debug.len(),
            "case_insensitive should find more matches.\nCS: {}\nCI: {}",
            cs_debug,
            ci_debug
        );
    }

    #[tokio::test]
    async fn test_ripgrep_fixed_strings_treats_regex_chars_literally() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("x.txt"), "a.b\naxb\n").unwrap();

        let tool = RipgrepTool::new(tmp.path().into());

        let regex_mode = tool
            .execute(serde_json::json!({ "pattern": "a.b" }))
            .await
            .unwrap();
        let regex_debug = format!("{}", regex_mode);

        let fixed_mode = tool
            .execute(serde_json::json!({ "pattern": "a.b", "fixed_strings": true }))
            .await
            .unwrap();
        let fixed_debug = format!("{}", fixed_mode);

        assert!(
            regex_debug.contains("axb") || regex_debug.len() > fixed_debug.len(),
            "regex mode should match more.\nregex: {}\nfixed: {}",
            regex_debug,
            fixed_debug
        );
    }

    #[tokio::test]
    async fn test_fd_finds_files_by_extension_filter() {
        if !binary_exists("fd") {
            eprintln!("skipping fd test — fd binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("keep.rs"), "").unwrap();
        std::fs::write(tmp.path().join("keep_too.rs"), "").unwrap();
        std::fs::write(tmp.path().join("skip.txt"), "").unwrap();
        std::fs::write(tmp.path().join("skip.md"), "").unwrap();

        let tool = FdTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({
                "pattern": ".",
                "extension": "rs"
            }))
            .await
            .unwrap();

        let files = result["files"].as_array().unwrap();
        let file_list: Vec<&str> = files.iter().filter_map(|v| v.as_str()).collect();
        for f in &file_list {
            assert!(
                f.ends_with(".rs"),
                "extension filter leaked non-.rs file: {}",
                f
            );
        }
        assert!(
            file_list.iter().any(|f| f.contains("keep")),
            "expected to find keep.rs, got: {:?}",
            file_list
        );
    }

    #[tokio::test]
    async fn test_fd_max_results_truncation() {
        if !binary_exists("fd") {
            eprintln!("skipping fd test — fd binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        for i in 0..15 {
            std::fs::write(tmp.path().join(format!("file{i:02}.log")), "").unwrap();
        }

        let tool = FdTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({
                "pattern": ".",
                "extension": "log",
                "max_results": 5,
            }))
            .await
            .unwrap();

        let count = result["count"].as_u64().unwrap();
        let total = result["total_found"].as_u64().unwrap();
        let truncated = result["truncated"].as_bool().unwrap();
        assert_eq!(count, 5, "max_results=5 must cap returned files");
        assert_eq!(total, 15, "total_found must reflect all 15 matches");
        assert!(truncated, "truncated flag must be true when total > max");
    }

    #[tokio::test]
    async fn test_fd_empty_result_has_correct_shape() {
        if !binary_exists("fd") {
            eprintln!("skipping fd test — fd binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("only.txt"), "").unwrap();

        let tool = FdTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({
                "pattern": "definitely_nothing_matches_xyz_abc_9999"
            }))
            .await
            .unwrap();

        assert_eq!(result["count"].as_u64().unwrap(), 0);
        assert_eq!(result["total_found"].as_u64().unwrap(), 0);
        assert!(!result["truncated"].as_bool().unwrap());
        assert!(result["files"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_rg_missing_pattern_returns_error() {
        if !binary_exists("rg") {
            eprintln!("skipping rg test — rg binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let result = tool.execute(serde_json::json!({})).await;
        assert!(
            result.is_err(),
            "missing pattern must return Err, got: {:?}",
            result
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("pattern"),
            "error must mention 'pattern', got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_sd_missing_required_params_returns_error() {
        if !binary_exists("sd") {
            eprintln!("skipping sd test — sd binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        let tool = SdTool::new(tmp.path().into());

        let r1 = tool
            .execute(serde_json::json!({"replace": "new", "path": "f"}))
            .await;
        assert!(r1.is_err(), "missing find must return Err");
        assert!(format!("{}", r1.unwrap_err()).contains("find"));

        let r2 = tool
            .execute(serde_json::json!({"find": "old", "path": "f"}))
            .await;
        assert!(r2.is_err(), "missing replace must return Err");
        assert!(format!("{}", r2.unwrap_err()).contains("replace"));

        let r3 = tool
            .execute(serde_json::json!({"find": "old", "replace": "new"}))
            .await;
        assert!(r3.is_err(), "missing path must return Err");
        assert!(format!("{}", r3.unwrap_err()).contains("path"));
    }

    #[tokio::test]
    async fn test_ripgrep_max_count_caps_matches() {
        if !binary_exists("rg") {
            eprintln!("skipping rg test — rg binary not on PATH");
            return;
        }
        let tmp = TempDir::new().unwrap();
        let mut content = String::new();
        for _ in 0..10 {
            content.push_str("MATCH_TOKEN\n");
        }
        std::fs::write(tmp.path().join("many.txt"), content).unwrap();

        let tool = RipgrepTool::new(tmp.path().into());
        let result = tool
            .execute(serde_json::json!({
                "pattern": "MATCH_TOKEN",
                "max_count": 3,
            }))
            .await
            .unwrap();

        let match_count = result["match_count"].as_u64().unwrap();
        assert!(
            match_count <= 3,
            "max_count=3 must limit results, got {}",
            match_count
        );
        assert!(match_count >= 1, "should find at least one match");
    }

    #[tokio::test]
    async fn test_glob_search_missing_pattern_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let err = tool
            .execute(serde_json::json!({}))
            .await
            .expect_err("glob_search without pattern must error");
        let msg = format!("{}", err);
        assert!(
            msg.contains("pattern required"),
            "error message should say 'pattern required', got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_grep_search_missing_pattern_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let err = tool
            .execute(serde_json::json!({}))
            .await
            .expect_err("grep_search without pattern must error");
        let msg = format!("{}", err);
        assert!(
            msg.contains("pattern required"),
            "error message should say 'pattern required', got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_glob_search_non_string_pattern_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let err = tool
            .execute(serde_json::json!({ "pattern": 42 }))
            .await
            .expect_err("glob_search with numeric pattern must error");
        let msg = format!("{}", err);
        assert!(msg.contains("pattern required"), "got: {}", msg);
    }
}
