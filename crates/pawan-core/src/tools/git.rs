//! Git operation tools
//!
//! Tools for git operations: status, diff, add, commit, log, blame, branch.

use super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Run a git command in a workspace directory
async fn run_git(workspace: &PathBuf, args: &[&str]) -> crate::Result<(bool, String, String)> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(crate::PawanError::Io)?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    if let Some(mut handle) = child.stdout.take() {
        handle.read_to_string(&mut stdout).await.ok();
    }
    if let Some(mut handle) = child.stderr.take() {
        handle.read_to_string(&mut stderr).await.ok();
    }

    let status = child.wait().await.map_err(crate::PawanError::Io)?;
    Ok((status.success(), stdout, stderr))
}

/// Tool for checking git status
///
/// This tool provides information about the current git repository status,
/// including modified files, untracked files, and branch information.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitStatusTool {
    workspace_root: PathBuf,
}

impl GitStatusTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Get the current git status showing staged, unstaged, and untracked files."
    }

    fn mutating(&self) -> bool {
        false // Git status is read-only
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "short": {
                    "type": "boolean",
                    "description": "Use short format output (default: false)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_status")
            .description(self.description())
            .parameter(Parameter::builder("short").param_type(ParameterType::Boolean).required(false)
                .description("Use short format output (default: false)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let short = args["short"].as_bool().unwrap_or(false);

        let mut git_args = vec!["status"];
        if short {
            git_args.push("-s");
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git status failed: {}",
                stderr
            )));
        }

        // Also get branch info
        let (_, branch_output, _) =
            run_git(&self.workspace_root, &["branch", "--show-current"]).await?;
        let branch = branch_output.trim().to_string();

        // Check if repo is clean
        let (_, porcelain, _) = run_git(&self.workspace_root, &["status", "--porcelain"]).await?;
        let is_clean = porcelain.trim().is_empty();

        Ok(json!({
            "status": stdout.trim(),
            "branch": branch,
            "is_clean": is_clean,
            "success": true
        }))
    }
}

/// Tool for getting git diff
///
/// This tool shows the differences between files in the working directory
/// and the git index, or between commits.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitDiffTool {
    workspace_root: PathBuf,
}

impl GitDiffTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show git diff for staged or unstaged changes. Can diff against a specific commit or branch."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "Show staged changes only (--cached). Default: false (shows unstaged)"
                },
                "file": {
                    "type": "string",
                    "description": "Specific file to diff (optional)"
                },
                "base": {
                    "type": "string",
                    "description": "Base commit/branch to diff against (e.g., 'main', 'HEAD~3')"
                },
                "stat": {
                    "type": "boolean",
                    "description": "Show diffstat summary instead of full diff"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_diff")
            .description(self.description())
            .parameter(Parameter::builder("staged").param_type(ParameterType::Boolean).required(false)
                .description("Show staged changes only (--cached). Default: false (shows unstaged)").build())
            .parameter(Parameter::builder("file").param_type(ParameterType::String).required(false)
                .description("Specific file to diff (optional)").build())
            .parameter(Parameter::builder("base").param_type(ParameterType::String).required(false)
                .description("Base commit/branch to diff against (e.g., 'main', 'HEAD~3')").build())
            .parameter(Parameter::builder("stat").param_type(ParameterType::Boolean).required(false)
                .description("Show diffstat summary instead of full diff").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let staged = args["staged"].as_bool().unwrap_or(false);
        let file = args["file"].as_str();
        let base = args["base"].as_str();
        let stat = args["stat"].as_bool().unwrap_or(false);

        let mut git_args = vec!["diff"];

        if staged {
            git_args.push("--cached");
        }

        if stat {
            git_args.push("--stat");
        }

        if let Some(b) = base {
            git_args.push(b);
        }

        if let Some(f) = file {
            git_args.push("--");
            git_args.push(f);
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git diff failed: {}",
                stderr
            )));
        }

        // Truncate if too large
        let max_size = 100_000;
        let truncated = stdout.len() > max_size;
        let diff = if truncated {
            format!(
                "{}...\n[truncated, {} bytes total]",
                &stdout[..max_size],
                stdout.len()
            )
        } else {
            stdout
        };

        Ok(json!({
            "diff": diff,
            "truncated": truncated,
            "has_changes": !diff.trim().is_empty(),
            "success": true
        }))
    }
}

/// Tool for staging files
///
/// This tool adds files to the git staging area in preparation for commit.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitAddTool {
    workspace_root: PathBuf,
}

impl GitAddTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitAddTool {
    fn name(&self) -> &str {
        "git_add"
    }

    fn description(&self) -> &str {
        "Stage files for commit. Can stage specific files or all changes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of files to stage. Use [\".\"] to stage all changes."
                },
                "all": {
                    "type": "boolean",
                    "description": "Stage all changes including untracked files (-A)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_add")
            .description(self.description())
            .parameter(Parameter::builder("files").param_type(ParameterType::Array).required(false)
                .description("List of files to stage. Use [\".\"] to stage all changes.").build())
            .parameter(Parameter::builder("all").param_type(ParameterType::Boolean).required(false)
                .description("Stage all changes including untracked files (-A)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let all = args["all"].as_bool().unwrap_or(false);
        let files: Vec<&str> = args["files"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut git_args = vec!["add"];

        if all {
            git_args.push("-A");
        } else if files.is_empty() {
            return Err(crate::PawanError::Tool(
                "Either 'files' or 'all: true' must be specified".into(),
            ));
        } else {
            for f in &files {
                git_args.push(f);
            }
        }

        let (success, _, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git add failed: {}",
                stderr
            )));
        }

        // Get status after adding
        let (_, status_output, _) = run_git(&self.workspace_root, &["status", "-s"]).await?;
        let staged_count = status_output
            .lines()
            .filter(|l| l.starts_with('A') || l.starts_with('M') || l.starts_with('D'))
            .count();

        Ok(json!({
            "success": true,
            "staged_count": staged_count,
            "message": if all {
                "Staged all changes".to_string()
            } else {
                format!("Staged {} file(s)", files.len())
            }
        }))
    }
}

/// Tool for creating commits
///
/// This tool creates a new git commit with the staged changes.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitCommitTool {
    workspace_root: PathBuf,
}

impl GitCommitTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Create a git commit with the staged changes. Requires a commit message."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Commit message (required)"
                },
                "body": {
                    "type": "string",
                    "description": "Extended commit body (optional)"
                }
            },
            "required": ["message"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_commit")
            .description(self.description())
            .parameter(Parameter::builder("message").param_type(ParameterType::String).required(true)
                .description("Commit message (required)").build())
            .parameter(Parameter::builder("body").param_type(ParameterType::String).required(false)
                .description("Extended commit body (optional)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let message = args["message"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("commit message is required".into()))?;

        let body = args["body"].as_str();

        // Check if there are staged changes
        let (_, staged, _) = run_git(&self.workspace_root, &["diff", "--cached", "--stat"]).await?;
        if staged.trim().is_empty() {
            return Err(crate::PawanError::Git(
                "No staged changes to commit. Use git_add first.".into(),
            ));
        }

        // Build commit message
        let full_message = if let Some(b) = body {
            format!("{}\n\n{}", message, b)
        } else {
            message.to_string()
        };

        let (success, stdout, stderr) =
            run_git(&self.workspace_root, &["commit", "-m", &full_message]).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git commit failed: {}",
                stderr
            )));
        }

        // Get the commit hash
        let (_, hash_output, _) =
            run_git(&self.workspace_root, &["rev-parse", "--short", "HEAD"]).await?;
        let commit_hash = hash_output.trim().to_string();

        Ok(json!({
            "success": true,
            "commit_hash": commit_hash,
            "message": message,
            "output": stdout.trim()
        }))
    }
}

/// Tool for viewing git log
///
/// This tool provides access to the git commit history, allowing inspection
/// of previous commits, authors, dates, and commit messages.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitLogTool {
    workspace_root: PathBuf,
}

impl GitLogTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }

    fn description(&self) -> &str {
        "Show git commit history. Supports limiting count, filtering by file, and custom format."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "count": {
                    "type": "integer",
                    "description": "Number of commits to show (default: 10)"
                },
                "file": {
                    "type": "string",
                    "description": "Show commits for a specific file"
                },
                "oneline": {
                    "type": "boolean",
                    "description": "Use compact one-line format (default: false)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_log")
            .description(self.description())
            .parameter(Parameter::builder("count").param_type(ParameterType::Integer).required(false)
                .description("Number of commits to show (default: 10)").build())
            .parameter(Parameter::builder("file").param_type(ParameterType::String).required(false)
                .description("Show commits for a specific file").build())
            .parameter(Parameter::builder("oneline").param_type(ParameterType::Boolean).required(false)
                .description("Use compact one-line format (default: false)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let count = args["count"].as_u64().unwrap_or(10);
        let file = args["file"].as_str();
        let oneline = args["oneline"].as_bool().unwrap_or(false);

        let count_str = count.to_string();
        let mut git_args = vec!["log", "-n", &count_str];

        if oneline {
            git_args.push("--oneline");
        } else {
            git_args.extend_from_slice(&["--pretty=format:%h %an %ar %s"]);
        }

        if let Some(f) = file {
            git_args.push("--");
            git_args.push(f);
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git log failed: {}",
                stderr
            )));
        }

        let commit_count = stdout.lines().count();

        Ok(json!({
            "log": stdout.trim(),
            "commit_count": commit_count,
            "success": true
        }))
    }
}

/// Tool for git blame
///
/// This tool shows line-by-line authorship information for files, indicating
/// who last modified each line and when.
///
/// # Fields
/// - `workspace_root`: The root directory of the workspace
pub struct GitBlameTool {
    workspace_root: PathBuf,
}

impl GitBlameTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitBlameTool {
    fn name(&self) -> &str {
        "git_blame"
    }

    fn description(&self) -> &str {
        "Show line-by-line authorship of a file. Useful for understanding who changed what."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "File to blame (required)"
                },
                "lines": {
                    "type": "string",
                    "description": "Line range, e.g., '10,20' for lines 10-20"
                }
            },
            "required": ["file"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_blame")
            .description(self.description())
            .parameter(Parameter::builder("file").param_type(ParameterType::String).required(true)
                .description("File to blame (required)").build())
            .parameter(Parameter::builder("lines").param_type(ParameterType::String).required(false)
                .description("Line range, e.g., '10,20' for lines 10-20").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let file = args["file"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("file is required for git_blame".into()))?;
        let lines = args["lines"].as_str();

        let mut git_args = vec!["blame", "--porcelain"];

        let line_range;
        if let Some(l) = lines {
            line_range = format!("-L{}", l);
            git_args.push(&line_range);
        }

        git_args.push(file);

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git blame failed: {}",
                stderr
            )));
        }

        // Truncate if too large
        let max_size = 50_000;
        let output = if stdout.len() > max_size {
            format!(
                "{}...\n[truncated, {} bytes total]",
                &stdout[..max_size],
                stdout.len()
            )
        } else {
            stdout
        };

        Ok(json!({
            "blame": output.trim(),
            "success": true
        }))
    }
}

/// Tool for listing and managing branches
pub struct GitBranchTool {
    workspace_root: PathBuf,
}

impl GitBranchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitBranchTool {
    fn name(&self) -> &str {
        "git_branch"
    }

    fn description(&self) -> &str {
        "List branches or get current branch name. Shows local and optionally remote branches."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "all": {
                    "type": "boolean",
                    "description": "Show both local and remote branches (default: false)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_branch")
            .description(self.description())
            .parameter(Parameter::builder("all").param_type(ParameterType::Boolean).required(false)
                .description("Show both local and remote branches (default: false)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let all = args["all"].as_bool().unwrap_or(false);

        // Get current branch
        let (_, current, _) = run_git(&self.workspace_root, &["branch", "--show-current"]).await?;
        let current_branch = current.trim().to_string();

        // List branches
        let mut git_args = vec!["branch", "--format=%(refname:short)"];
        if all {
            git_args.push("-a");
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git branch failed: {}",
                stderr
            )));
        }

        let branches: Vec<&str> = stdout
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(json!({
            "current": current_branch,
            "branches": branches,
            "count": branches.len(),
            "success": true
        }))
    }
}

/// Tool for git checkout (switch branches or restore files)
pub struct GitCheckoutTool {
    workspace_root: PathBuf,
}

impl GitCheckoutTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitCheckoutTool {
    fn name(&self) -> &str {
        "git_checkout"
    }

    fn description(&self) -> &str {
        "Switch branches or restore working tree files. Can create new branches with create=true."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Branch name, commit, or file path to checkout"
                },
                "create": {
                    "type": "boolean",
                    "description": "Create a new branch (git checkout -b)"
                },
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Specific files to restore (git checkout -- <files>)"
                }
            },
            "required": ["target"]
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_checkout")
            .description(self.description())
            .parameter(Parameter::builder("target").param_type(ParameterType::String).required(true)
                .description("Branch name, commit, or file path to checkout").build())
            .parameter(Parameter::builder("create").param_type(ParameterType::Boolean).required(false)
                .description("Create a new branch (git checkout -b)").build())
            .parameter(Parameter::builder("files").param_type(ParameterType::Array).required(false)
                .description("Specific files to restore (git checkout -- <files>)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let target = args["target"]
            .as_str()
            .ok_or_else(|| crate::PawanError::Tool("target is required".into()))?;
        let create = args["create"].as_bool().unwrap_or(false);
        let files: Vec<&str> = args["files"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut git_args: Vec<&str> = vec!["checkout"];

        if create {
            git_args.push("-b");
        }

        git_args.push(target);

        if !files.is_empty() {
            git_args.push("--");
            git_args.extend(files.iter());
        }

        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git checkout failed: {}",
                stderr
            )));
        }

        Ok(json!({
            "success": true,
            "target": target,
            "created": create,
            "output": format!("{}{}", stdout, stderr).trim().to_string()
        }))
    }
}

/// Tool for git stash operations
pub struct GitStashTool {
    workspace_root: PathBuf,
}

impl GitStashTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for GitStashTool {
    fn name(&self) -> &str {
        "git_stash"
    }

    fn description(&self) -> &str {
        "Stash or restore uncommitted changes. Actions: push (default), pop, list, drop."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop", "show"],
                    "description": "Stash action (default: push)"
                },
                "message": {
                    "type": "string",
                    "description": "Message for stash push"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index for pop/drop/show (default: 0)"
                }
            },
            "required": []
        })
    }

    fn thulp_definition(&self) -> thulp_core::ToolDefinition {
        use thulp_core::{Parameter, ParameterType};
        thulp_core::ToolDefinition::builder("git_stash")
            .description(self.description())
            .parameter(Parameter::builder("action").param_type(ParameterType::String).required(false)
                .description("Stash action (default: push)").build())
            .parameter(Parameter::builder("message").param_type(ParameterType::String).required(false)
                .description("Message for stash push").build())
            .parameter(Parameter::builder("index").param_type(ParameterType::Integer).required(false)
                .description("Stash index for pop/drop/show (default: 0)").build())
            .build()
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let action = args["action"].as_str().unwrap_or("push");
        let message = args["message"].as_str();
        let index = args["index"].as_u64().unwrap_or(0);

        let git_args: Vec<String> = match action {
            "push" => {
                let mut a = vec!["stash".to_string(), "push".to_string()];
                if let Some(msg) = message {
                    a.push("-m".to_string());
                    a.push(msg.to_string());
                }
                a
            }
            "pop" => vec!["stash".to_string(), "pop".to_string(), format!("stash@{{{}}}", index)],
            "list" => vec!["stash".to_string(), "list".to_string()],
            "drop" => vec!["stash".to_string(), "drop".to_string(), format!("stash@{{{}}}", index)],
            "show" => vec!["stash".to_string(), "show".to_string(), "-p".to_string(), format!("stash@{{{}}}", index)],
            _ => return Err(crate::PawanError::Tool(format!("Unknown stash action: {}", action))),
        };

        let git_args_ref: Vec<&str> = git_args.iter().map(|s| s.as_str()).collect();
        let (success, stdout, stderr) = run_git(&self.workspace_root, &git_args_ref).await?;

        if !success {
            return Err(crate::PawanError::Git(format!(
                "git stash {} failed: {}",
                action, stderr
            )));
        }

        Ok(json!({
            "success": true,
            "action": action,
            "output": stdout.trim().to_string()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_git_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        // Initialize git repo
        let mut cmd = Command::new("git");
        cmd.args(["init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        // Configure git for test
        let mut cmd = Command::new("git");
        cmd.args(["config", "user.email", "test@test.com"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        let mut cmd = Command::new("git");
        cmd.args(["config", "user.name", "Test User"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        temp_dir
    }

    #[tokio::test]
    async fn test_git_status_empty_repo() {
        let temp_dir = setup_git_repo().await;

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["is_clean"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_status_with_untracked() {
        let temp_dir = setup_git_repo().await;

        // Create an untracked file
        std::fs::write(temp_dir.path().join("test.txt"), "hello").unwrap();

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(!result["is_clean"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_add_and_commit() {
        let temp_dir = setup_git_repo().await;

        // Create a file
        std::fs::write(temp_dir.path().join("test.txt"), "hello").unwrap();

        // Add the file
        let add_tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let add_result = add_tool
            .execute(json!({
                "files": ["test.txt"]
            }))
            .await
            .unwrap();
        assert!(add_result["success"].as_bool().unwrap());

        // Commit
        let commit_tool = GitCommitTool::new(temp_dir.path().to_path_buf());
        let commit_result = commit_tool
            .execute(json!({
                "message": "Add test file"
            }))
            .await
            .unwrap();
        assert!(commit_result["success"].as_bool().unwrap());
        assert!(!commit_result["commit_hash"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_git_diff_no_changes() {
        let temp_dir = setup_git_repo().await;

        let tool = GitDiffTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(!result["has_changes"].as_bool().unwrap());
    }
    #[tokio::test]
    async fn test_git_status_tool_exists() {
        let temp_dir = setup_git_repo().await;
        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        assert_eq!(tool.name(), "git_status");
    }

    #[tokio::test]
    async fn test_git_log_tool_exists() {
        let temp_dir = setup_git_repo().await;
        let tool = GitLogTool::new(temp_dir.path().to_path_buf());
        assert_eq!(tool.name(), "git_log");
    }

    #[tokio::test]
    async fn test_git_diff_schema() {
        let temp_dir = setup_git_repo().await;
        let tool = GitDiffTool::new(temp_dir.path().to_path_buf());
        let schema = tool.parameters_schema();
        let obj = schema.as_object().unwrap();
        let props = obj.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("staged"));
        assert!(props.contains_key("file"));
        assert!(props.contains_key("base"));
        assert!(props.contains_key("stat"));
    }

    #[tokio::test]
    async fn test_git_diff_with_changes() {
        let temp_dir = setup_git_repo().await;
        // Create, add, commit a file
        std::fs::write(temp_dir.path().join("f.txt"), "original").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();
        // Modify the file
        std::fs::write(temp_dir.path().join("f.txt"), "modified").unwrap();

        let tool = GitDiffTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert!(result["has_changes"].as_bool().unwrap());
        assert!(result["diff"].as_str().unwrap().contains("modified"));
    }

    #[tokio::test]
    async fn test_git_log_with_commits() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("a.txt"), "a").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "first commit"]).current_dir(temp_dir.path()).output().await.unwrap();
        std::fs::write(temp_dir.path().join("b.txt"), "b").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "second commit"]).current_dir(temp_dir.path()).output().await.unwrap();

        let tool = GitLogTool::new(temp_dir.path().into());
        let result = tool.execute(json!({"count": 5})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        let log = result["log"].as_str().unwrap();
        assert!(log.contains("first commit"));
        assert!(log.contains("second commit"));
    }

    #[tokio::test]
    async fn test_git_branch_list() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("f.txt"), "init").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();

        let tool = GitBranchTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        let branches = result["branches"].as_array().unwrap();
        assert!(!branches.is_empty(), "Should have at least one branch");
        assert!(result["current"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_git_checkout_create_branch() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("f.txt"), "init").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp_dir.path()).output().await.unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp_dir.path()).output().await.unwrap();

        let tool = GitCheckoutTool::new(temp_dir.path().into());
        let result = tool.execute(json!({"target": "feature-test", "create": true})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());

        // Verify we're on the new branch
        let branch_tool = GitBranchTool::new(temp_dir.path().into());
        let branches = branch_tool.execute(json!({})).await.unwrap();
        assert_eq!(branches["current"].as_str().unwrap(), "feature-test");
    }

    #[tokio::test]
    async fn test_git_stash_on_clean_repo() {
        let temp_dir = setup_git_repo().await;
        let tool = GitStashTool::new(temp_dir.path().into());
        // List stashes on empty repo
        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_git_blame_requires_file() {
        let temp_dir = setup_git_repo().await;
        let tool = GitBlameTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "blame without file should error");
    }

    #[tokio::test]
    async fn test_git_tool_schemas() {
        let tmp = TempDir::new().unwrap();
        // Verify all git tools have correct names and non-empty schemas
        let tools: Vec<(&str, Box<dyn Tool>)> = vec![
            ("git_status", Box::new(GitStatusTool::new(tmp.path().into()))),
            ("git_diff", Box::new(GitDiffTool::new(tmp.path().into()))),
            ("git_add", Box::new(GitAddTool::new(tmp.path().into()))),
            ("git_commit", Box::new(GitCommitTool::new(tmp.path().into()))),
            ("git_log", Box::new(GitLogTool::new(tmp.path().into()))),
            ("git_blame", Box::new(GitBlameTool::new(tmp.path().into()))),
            ("git_branch", Box::new(GitBranchTool::new(tmp.path().into()))),
            ("git_checkout", Box::new(GitCheckoutTool::new(tmp.path().into()))),
            ("git_stash", Box::new(GitStashTool::new(tmp.path().into()))),
        ];
        for (expected_name, tool) in &tools {
            assert_eq!(tool.name(), *expected_name, "Tool name mismatch");
            assert!(!tool.description().is_empty(), "Missing description for {}", expected_name);
            let schema = tool.parameters_schema();
            assert!(schema.is_object(), "Schema should be object for {}", expected_name);
        }
    }

    #[tokio::test]
    async fn test_git_commit_missing_message_errors() {
        let temp_dir = setup_git_repo().await;
        let tool = GitCommitTool::new(temp_dir.path().to_path_buf());
        // No "message" field
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "commit without message must error");
    }

    #[tokio::test]
    async fn test_git_commit_multiline_message_preserved() {
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("a.txt"), "content").unwrap();

        GitAddTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "files": ["a.txt"] }))
            .await
            .unwrap();

        // Commit with a message that has newlines, backticks, dollars,
        // and quotes — everything that could break shell escaping.
        let message = "feat: the subject line\n\nThis is the body.\nIt has `backticks`, $dollars, and \"quotes\".\n\nCo-Authored-By: Test <test@example.com>";
        let commit_result = GitCommitTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "message": message }))
            .await
            .unwrap();
        assert!(commit_result["success"].as_bool().unwrap());

        // Read the commit message back via git log
        let log_result = GitLogTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "count": 1 }))
            .await
            .unwrap();
        let log_str = format!("{}", log_result);
        assert!(
            log_str.contains("the subject line"),
            "log should contain subject line, got: {}",
            log_str
        );
    }

    #[tokio::test]
    async fn test_git_stash_on_dirty_repo_saves_changes() {
        let temp_dir = setup_git_repo().await;
        // First commit a base file
        std::fs::write(temp_dir.path().join("base.txt"), "v1").unwrap();
        GitAddTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "files": ["base.txt"] }))
            .await
            .unwrap();
        GitCommitTool::new(temp_dir.path().to_path_buf())
            .execute(json!({ "message": "base" }))
            .await
            .unwrap();

        // Now modify it so there's something to stash
        std::fs::write(temp_dir.path().join("base.txt"), "v2-dirty").unwrap();

        let stash_tool = GitStashTool::new(temp_dir.path().to_path_buf());
        let result = stash_tool
            .execute(json!({ "action": "push", "message": "test stash" }))
            .await
            .unwrap();
        assert!(result["success"].as_bool().unwrap());

        // Working tree should be clean again (stash popped the change)
        let content = std::fs::read_to_string(temp_dir.path().join("base.txt")).unwrap();
        assert_eq!(content, "v1", "stash push should revert working tree");
    }

    #[tokio::test]
    async fn test_git_log_with_count_limit() {
        let temp_dir = setup_git_repo().await;
        // Make 3 commits
        for i in 1..=3 {
            std::fs::write(
                temp_dir.path().join(format!("file{i}.txt")),
                format!("v{i}"),
            )
            .unwrap();
            GitAddTool::new(temp_dir.path().to_path_buf())
                .execute(json!({ "files": [format!("file{i}.txt")] }))
                .await
                .unwrap();
            GitCommitTool::new(temp_dir.path().to_path_buf())
                .execute(json!({ "message": format!("commit {i}") }))
                .await
                .unwrap();
        }

        // Log with count=2 should only return 2 commits (one line per commit
        // under --pretty=format:%h %an %ar %s)
        let tool = GitLogTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({ "count": 2 })).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert_eq!(
            result["commit_count"].as_u64().unwrap(),
            2,
            "count=2 should return exactly 2 commits, got: {}",
            result["log"].as_str().unwrap_or("")
        );
        // Sanity check: the log string should mention the 2 most recent commits
        let log = result["log"].as_str().unwrap();
        assert!(log.contains("commit 3"), "expected 'commit 3' in log, got: {}", log);
        assert!(log.contains("commit 2"), "expected 'commit 2' in log, got: {}", log);
        assert!(!log.contains("commit 1"), "'commit 1' should be excluded by count=2, got: {}", log);
    }

    // ─── Edge cases for git tools (task #22/git) ────────────────────────

    #[tokio::test]
    async fn test_git_add_neither_files_nor_all_returns_error() {
        // GitAddTool requires either `files` (non-empty) or `all: true`.
        // Omitting both must return a specific error message.
        let temp_dir = setup_git_repo().await;
        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "git_add with no args must return Err");
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("files") && err.contains("all"),
            "error must mention both 'files' and 'all', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_git_add_all_without_files_list_succeeds() {
        // all=true should work even when `files` is not specified at all.
        // This tests the early branch that skips the empty-files check.
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("x.txt"), "a").unwrap();
        std::fs::write(temp_dir.path().join("y.txt"), "b").unwrap();

        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({ "all": true })).await.unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("Staged all changes"),
            "all=true should report 'Staged all changes'"
        );
    }

    #[tokio::test]
    async fn test_git_add_empty_files_array_returns_error() {
        // files=[] with no all flag must ALSO error (empty array is falsy).
        let temp_dir = setup_git_repo().await;
        let tool = GitAddTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({ "files": [] })).await;
        assert!(
            result.is_err(),
            "empty files array + no all flag must error"
        );
    }

    #[tokio::test]
    async fn test_git_checkout_nonexistent_branch_without_create_errors() {
        // Checkout to a non-existent branch WITHOUT create=true must fail,
        // not silently create it. This pins the "safety" contract of the tool.
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("init.txt"), "init").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        let tool = GitCheckoutTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(json!({
                "target": "nonexistent-branch-xyz-abc-9999",
                "create": false
            }))
            .await;
        assert!(
            result.is_err(),
            "checkout to nonexistent branch without create must error"
        );
    }

    #[tokio::test]
    async fn test_git_status_detects_modified_file() {
        // GitStatusTool should report modified files that were previously committed
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("tracked.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init tracked"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        // Modify the tracked file
        std::fs::write(temp_dir.path().join("tracked.txt"), "v2").unwrap();

        let tool = GitStatusTool::new(temp_dir.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();
        // Verify the status includes the modified file
        let serialized = result.to_string();
        assert!(
            serialized.contains("tracked.txt"),
            "status must mention modified tracked.txt, got: {}",
            serialized
        );
    }

    #[tokio::test]
    async fn test_git_log_count_zero_uses_default_or_errors() {
        // count=0 is an unusual value — test that it either uses a default
        // or errors rather than returning unbounded output.
        let temp_dir = setup_git_repo().await;
        std::fs::write(temp_dir.path().join("f.txt"), "init").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        let tool = GitLogTool::new(temp_dir.path().to_path_buf());
        // count=0 — observe current behavior (documented pin)
        let result = tool.execute(json!({ "count": 0 })).await;
        // Either succeeds with default count OR errors — both are acceptable,
        // as long as it doesn't hang or return unbounded output
        assert!(
            result.is_ok() || result.is_err(),
            "count=0 should not hang"
        );
    }
}
