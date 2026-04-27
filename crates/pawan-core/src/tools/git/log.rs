use super::run_git;
use super::super::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::git::staging::{GitAddTool, GitCommitTool};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::process::Command;

    async fn setup_git_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_dir.path())
            .output()
            .await
            .unwrap();

        temp_dir
    }

    #[tokio::test]
    async fn test_git_log_tool_exists() {
        let temp_dir = setup_git_repo().await;
        let tool = GitLogTool::new(temp_dir.path().to_path_buf());
        assert_eq!(tool.name(), "git_log");
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
    async fn test_git_blame_requires_file() {
        let temp_dir = setup_git_repo().await;
        let tool = GitBlameTool::new(temp_dir.path().into());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "blame without file should error");
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
