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
        // Check that schema has the expected parameters
        let obj = schema.as_object().unwrap();
        let props = obj.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("staged"));
        assert!(props.contains_key("file"));
        assert!(props.contains_key("base"));
        assert!(props.contains_key("stat"));
    }
}
